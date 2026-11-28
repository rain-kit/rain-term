import { FileUp, KeyRound, Loader2, Lock, Pencil, Sparkles, Trash2, UserPlus, UserRound } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { accountApi } from "../lib/api";
import { formatTime, KIND_LABEL, keyTypeLabel } from "../lib/format";
import { useI18n } from "../lib/i18n";
import { useAccounts, useConns, useKeys, useToast } from "../lib/store";
import type { AccountAuth, AccountScope, AccountView, ConnKind, ConnView, SshKey } from "../lib/types";
import KeyImportDialog, { KeyGenerateDialog } from "./KeyImportDialog";
import KindIcon from "./KindIcon";
import { Button, Card, Confirm, ContextMenu, cx, Dialog, Empty, Field, IconButton, Input, PasswordInput, Segmented, Select, type SelectOption } from "./ui";

/** 登录用户能否用于这种连接。 */
export const accountAllows = (a: { scope: AccountScope }, kind: ConnKind) => a.scope === "all" || a.scope === kind;

/** 使用这个登录用户的连接。 */
export const accountUsers = (conns: ConnView[], id: string) => conns.filter((c) => c.accountId === id);

/** 下拉框里显示的名称：名称和用户名不同时两个都显示。 */
export const accountLabel = (a: AccountView) => (a.name === a.user ? a.user : `${a.name} · ${a.user}`);

const scopeBadgeCls: Record<AccountScope, string> = {
  all: "bg-subtle text-muted",
  ssh: "bg-slate-500/12 text-slate-600 dark:text-slate-300",
  rdp: "bg-sky-500/12 text-sky-600 dark:text-sky-400",
};

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
export function AccountEditor({
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
  const [generating, setGenerating] = useState(false);

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

  const keyOptions = useMemo<SelectOption[]>(() => {
    const opts: SelectOption[] = keys.map((k) => ({ value: k.id, label: k.name, sub: keyTypeLabel(k) }));
    if (f.keyId && !keys.some((k) => k.id === f.keyId)) opts.push({ value: f.keyId, label: t("editor.keyMissing") });
    return [{ value: "", label: t(keys.length ? "editor.keyNone" : "editor.keyEmpty") }, ...opts];
  }, [keys, f.keyId, t]);

  const submit = async () => {
    if (busy) return;
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
      divided
      title={
        <span className="flex items-center gap-2">
          <span className="flex size-6 items-center justify-center rounded-md bg-accent/12 text-accent">
            <UserRound size={14} />
          </span>
          {t(account ? "accounts.editTitle" : "accounts.newTitle")}
        </span>
      }
      width="max-w-md"
      footer={
        <>
          <Button onClick={onClose} disabled={busy}>
            {t("common.cancel")}
          </Button>
          <Button variant="primary" icon={busy ? <Loader2 size={14} className="animate-spin" /> : undefined} onClick={submit} disabled={busy}>
            {t("common.save")}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-3.5">
        <div className="flex gap-3">
          <Field label={t("editor.user")} error={errors.user} className="flex-1">
            <Input
              autoFocus
              value={f.user}
              invalid={!!errors.user}
              onChange={(e) => set("user", e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && submit()}
            />
          </Field>
          <Field label={t("accounts.name")} className="flex-1">
            <Input
              placeholder={t("accounts.namePlaceholder")}
              value={f.name}
              onChange={(e) => set("name", e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && submit()}
            />
          </Field>
        </div>
        <p className="-mt-2 text-xs text-muted">{t("accounts.userHint")}</p>

        <Field label={t("editor.authType")}>
          <Segmented<AccountAuth>
            value={f.auth}
            onChange={(v) => set("auth", v)}
            className="flex w-full [&>button]:flex-1 [&>button]:justify-center"
            options={[
              {
                value: "password",
                label: (
                  <>
                    <Lock size={13} />
                    {t("editor.auth.password")}
                  </>
                ),
              },
              {
                value: "key",
                label: (
                  <>
                    <KeyRound size={13} />
                    {t("editor.auth.key")}
                  </>
                ),
              },
            ]}
          />
        </Field>

        {f.auth === "password" ? (
          <Field label={t("editor.password")} hint={saved ? t("editor.secretSaved") : undefined}>
            <PasswordInput
              value={f.password}
              placeholder={saved ? "••••••••" : undefined}
              onChange={(e) => set("password", e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && submit()}
            />
          </Field>
        ) : (
          <Field label={t("editor.key")} error={errors.keyId}>
            <div className="flex gap-1.5">
              <Select className="min-w-0 flex-1" value={f.keyId} invalid={!!errors.keyId} onChange={(v) => set("keyId", v)} options={keyOptions} />
              <Button type="button" title={t("keys.importTitle")} icon={<FileUp size={14} />} onClick={() => setImporting(true)}>
                {t("keys.import")}
              </Button>
              <Button type="button" title={t("keys.generateTitle")} icon={<Sparkles size={14} />} onClick={() => setGenerating(true)}>
                {t("keys.generate")}
              </Button>
            </div>
          </Field>
        )}

        <Field label={t("accounts.scope")} hint={f.auth === "password" ? t("accounts.scopeHint") : undefined}>
          {f.auth === "key" ? (
            <div className="flex h-8.5 items-center gap-2 rounded-lg border border-line bg-subtle/50 px-3 text-xs text-muted">
              <KindIcon kind="ssh" className="!size-5" />
              <span>{t("accounts.scopeKeyHint")}</span>
            </div>
          ) : (
            <Segmented<AccountScope>
              value={f.scope}
              onChange={(v) => set("scope", v)}
              className="flex w-full [&>button]:flex-1 [&>button]:justify-center"
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
      <KeyGenerateDialog open={generating} onClose={() => setGenerating(false)} onDone={(k: SshKey) => set("keyId", k.id)} />
    </Dialog>
  );
}

/** 密钥窗口的“用户”页：新建、编辑和删除登录用户。 */
export default function AccountsPanel({
  query = "",
  creating,
  onCreateOpen,
  onCreateClose,
  onAdded,
}: {
  query?: string;
  creating: boolean;
  onCreateOpen: () => void;
  onCreateClose: () => void;
  onAdded?: (a: AccountView) => void;
}) {
  const { t, tx } = useI18n();
  const toast = useToast();
  const { accounts } = useAccounts();
  const { keys } = useKeys();
  const { conns } = useConns();
  const [editing, setEditing] = useState<AccountView | null>(null);
  const [deleting, setDeleting] = useState<AccountView | null>(null);
  const [refInfo, setRefInfo] = useState<{ mode: "inUse" | "usage"; account: AccountView; users: ConnView[] } | null>(null);
  const [ctxMenu, setCtxMenu] = useState<{ at: { x: number; y: number }; account: AccountView } | null>(null);

  const users = useMemo(() => new Map(accounts.map((a) => [a.id, accountUsers(conns, a.id)])), [accounts, conns]);
  const keyMap = useMemo(() => new Map(keys.map((k) => [k.id, k])), [keys]);

  const filtered = useMemo(() => {
    const s = query.trim().toLowerCase();
    if (!s) return accounts;
    return accounts.filter((a) => [a.name, a.user, keyMap.get(a.keyId ?? "")?.name ?? ""].some((v) => v.toLowerCase().includes(s)));
  }, [accounts, query, keyMap]);

  const askDelete = (a: AccountView) => {
    const list = users.get(a.id) ?? [];
    if (list.length) setRefInfo({ mode: "inUse", account: a, users: list });
    else setDeleting(a);
  };

  return (
    <>
      {accounts.length === 0 ? (
        <Empty
          compact
          icon={<UserRound size={22} />}
          title={t("accounts.empty")}
          desc={t("accounts.emptyDesc")}
          action={
            <Button size="sm" variant="primary" icon={<UserPlus size={14} />} onClick={onCreateOpen}>
              {t("accounts.newTitle")}
            </Button>
          }
        />
      ) : filtered.length === 0 ? (
        <div className="py-12 text-center text-xs text-muted">{t("accounts.noMatch")}</div>
      ) : (
        <Card className="divide-y divide-line overflow-hidden">
          {filtered.map((a) => {
            const list = users.get(a.id) ?? [];
            const count = list.length;
            const key = a.keyId ? keyMap.get(a.keyId) : undefined;
            const diffName = a.name !== a.user;
            return (
              <div
                key={a.id}
                onDoubleClick={() => setEditing(a)}
                onContextMenu={(e) => {
                  e.preventDefault();
                  setCtxMenu({ at: { x: e.clientX, y: e.clientY }, account: a });
                }}
                className="group flex items-center gap-3 px-3.5 py-2.5 transition-colors hover:bg-subtle/60"
              >
                <div className="flex size-9 shrink-0 items-center justify-center rounded-xl bg-accent/10 text-accent">
                  <UserRound size={17} />
                </div>
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-1.5">
                    <span className="truncate text-sm font-medium">{a.name}</span>
                    {diffName && <span className="truncate font-mono text-xs text-muted">@{a.user}</span>}
                    <span className={cx("shrink-0 rounded px-1.5 text-[10px] leading-4 font-medium", scopeBadgeCls[a.scope])}>
                      {a.scope === "all" ? t("accounts.scope.all") : KIND_LABEL[a.scope]}
                    </span>
                  </div>
                  <div className="mt-0.5 flex items-center gap-1.5 truncate text-[11px] text-muted">
                    {a.auth === "key" ? (
                      <span className={cx("inline-flex items-center gap-1 truncate", !key && "text-red-600 dark:text-red-400")}>
                        <KeyRound size={11} className="shrink-0" />
                        <span className="truncate">{key ? key.name : t("editor.keyMissing")}</span>
                      </span>
                    ) : (
                      <span className="inline-flex items-center gap-1">
                        <Lock size={11} className="shrink-0" />
                        <span>{t(a.hasPassword ? "editor.auth.password" : "accounts.noPassword")}</span>
                      </span>
                    )}
                    {a.createdAt && (
                      <>
                        <span className="opacity-50">·</span>
                        <span className="shrink-0 tabular-nums opacity-80">{formatTime(Date.parse(a.createdAt) / 1000)}</span>
                      </>
                    )}
                  </div>
                </div>
                <div className="shrink-0">
                  {count > 0 ? (
                    <button
                      type="button"
                      title={list.map((u) => u.name).join("、")}
                      onClick={() => setRefInfo({ mode: "usage", account: a, users: list })}
                      className="rounded-md bg-accent/10 px-2 py-0.5 text-xs font-medium text-accent transition-colors hover:bg-accent/18"
                    >
                      {t("keys.usedBy", { count })}
                    </button>
                  ) : (
                    <span className="px-1.5 text-xs text-muted/70">{t("keys.unused")}</span>
                  )}
                </div>
                <div className="flex shrink-0 items-center gap-0.5">
                  <IconButton label={t("accounts.edit")} size="sm" onClick={() => setEditing(a)}>
                    <Pencil size={14} />
                  </IconButton>
                  <IconButton label={t("common.delete")} size="sm" tone="danger" onClick={() => askDelete(a)}>
                    <Trash2 size={14} />
                  </IconButton>
                </div>
              </div>
            );
          })}
        </Card>
      )}

      <ContextMenu
        at={ctxMenu?.at ?? null}
        items={
          ctxMenu
            ? [
                { label: t("accounts.edit"), icon: <Pencil size={14} />, onClick: () => setEditing(ctxMenu.account) },
                { label: t("common.delete"), icon: <Trash2 size={14} />, danger: true, divider: true, onClick: () => askDelete(ctxMenu.account) },
              ]
            : []
        }
        onClose={() => setCtxMenu(null)}
      />

      <AccountEditor open={creating} account={null} onClose={onCreateClose} onDone={onAdded} />
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
        open={!!refInfo}
        onClose={() => setRefInfo(null)}
        title={t(refInfo?.mode === "usage" ? "keys.usageTitle" : "conn.inUseTitle")}
        width="max-w-sm"
        footer={<Button onClick={() => setRefInfo(null)}>{t("common.close")}</Button>}
      >
        <div className="text-sm text-muted">
          {tx(refInfo?.mode === "usage" ? "accounts.usageMessage" : "accounts.inUseMessage", {
            name: <span className="font-medium text-ink">{refInfo?.account.name}</span>,
          })}
        </div>
        <ul className="mt-2.5 mb-1 flex max-h-48 flex-col gap-1.5 overflow-y-auto">
          {refInfo?.users.map((u) => (
            <li key={u.id} className="flex items-center gap-2 rounded-lg bg-subtle/60 px-2.5 py-1.5 text-sm">
              <KindIcon kind={u.kind} />
              <span className="truncate">{u.name}</span>
            </li>
          ))}
        </ul>
      </Dialog>
    </>
  );
}
