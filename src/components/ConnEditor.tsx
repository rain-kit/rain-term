import { Check, KeyRound, Loader2, Lock, PlugZap, Plus, Settings2, SlidersHorizontal, UserRound, Waypoints } from "lucide-react";
import { useEffect, useMemo, useState, type KeyboardEvent } from "react";
import { connApi } from "../lib/api";
import { loadGroups } from "../lib/groups";
import {
  CONN_COLORS,
  connAddress,
  DEFAULT_PORT,
  DEFAULT_USER,
  KIND_LABEL,
  keyTypeLabel,
  RDP_MAX_SIZE,
  RDP_MIN_SIZE,
  type ConnColor,
} from "../lib/format";
import { t, useI18n } from "../lib/i18n";
import { useAccounts, useConns, useKeys, useSettings, useToast } from "../lib/store";
import type { AccountAuth, AccountView, ConnKind, ConnView, RdpResolution, SaveConnReq, SftpSudo } from "../lib/types";
import { accountAllows, AccountEditor } from "./AccountsPanel";
import KeysDialog, { type KeysTab } from "./KeysDialog";
import KindIcon from "./KindIcon";
import {
  Button,
  Card,
  ComboInput,
  cx,
  Dialog,
  Field,
  IncognitoBadge,
  Input,
  PasswordInput,
  ProdBadge,
  Segmented,
  Select,
  SwitchRow,
  type SelectOption,
} from "./ui";

type AuthMode = AccountAuth | "account";
type EditorTab = "general" | "advanced";

interface Form {
  kind: ConnKind;
  name: string;
  group: string;
  color: string;
  note: string;
  production: boolean;
  host: string;
  port: string;
  /** 认证方式选“登录用户”时使用的用户 ID，选了之后不用填用户名、密码和私钥。 */
  accountId: string;
  user: string;
  password: string;
  jumpId: string;
  auth: AuthMode;
  // SSH
  incognito: boolean;
  keyId: string;
  keepaliveSeconds: string;
  initDir: string;
  sftpSudo: SftpSudo;
  // 远程桌面
  resolution: RdpResolution;
  width: string;
  height: string;
  clipboard: boolean;
}

type Errors = Partial<Record<keyof Form, string>>;

const DEFAULT_WIDTH = 1920;
const DEFAULT_HEIGHT = 1080;
const RDP_PRESETS: [number, number][] = [
  [1920, 1080],
  [2560, 1440],
  [1280, 720],
];

function toForm(c?: ConnView | null): Form {
  const kind = c?.kind ?? "ssh";
  const ssh = c?.kind === "ssh" ? c : undefined;
  const rdp = c?.kind === "rdp" ? c : undefined;
  return {
    kind,
    name: c?.name ?? "",
    group: c?.group ?? "",
    color: c?.color ?? "",
    note: c?.note ?? "",
    production: !!c?.production,
    host: c?.host ?? "",
    port: String(c?.port ?? DEFAULT_PORT[kind]),
    accountId: c?.accountId ?? "",
    user: c?.user ?? DEFAULT_USER[kind],
    password: "",
    jumpId: c?.jumpId ?? "",
    incognito: !!ssh?.incognito,
    // 新建时默认用登录用户；选了登录用户时认证方式切到“登录用户”，改回其他方式时不带入登录用户的密钥。
    // 编辑器不再提供 SSH Agent，旧连接编辑时按新建处理，需要重新选择认证方式。
    auth: !c || c.accountId || ssh?.auth === "agent" ? "account" : ssh?.auth ?? "password",
    keyId: (!c?.accountId && ssh?.keyId) || "",
    keepaliveSeconds: String(ssh?.keepaliveSeconds ?? 30),
    initDir: ssh?.initDir ?? "",
    sftpSudo: ssh?.sftpSudo ?? "default",
    resolution: rdp?.resolution ?? "fit",
    width: String(rdp?.width ?? DEFAULT_WIDTH),
    height: String(rdp?.height ?? DEFAULT_HEIGHT),
    clipboard: rdp?.clipboard ?? true,
  };
}

const isInt = (v: string, min: number, max: number) => /^\d+$/.test(v.trim()) && Number(v) >= min && Number(v) <= max;
const isSize = (v: string) => isInt(v, RDP_MIN_SIZE, RDP_MAX_SIZE);

function validate(f: Form): Errors {
  const e: Errors = {};
  if (!f.host.trim()) e.host = t("editor.required");
  if (!isInt(f.port, 1, 65535)) e.port = t("editor.err.port");
  if (f.auth === "account") {
    if (!f.accountId) e.accountId = t("editor.err.accountRequired");
  } else if (!f.user.trim()) {
    e.user = t("editor.required");
  }
  if (f.kind === "ssh") {
    if (f.auth === "key" && !f.keyId) e.keyId = t("editor.err.keyRequired");
    if (!isInt(f.keepaliveSeconds, 0, 3600)) e.keepaliveSeconds = t("editor.err.keepalive");
  } else if (f.resolution === "fixed") {
    const msg = t("editor.err.rdpSize", { min: RDP_MIN_SIZE, max: RDP_MAX_SIZE });
    if (!isSize(f.width)) e.width = msg;
    if (!isSize(f.height)) e.height = msg;
  }
  return e;
}

function toReq(f: Form, id: string): SaveConnReq {
  // 选了登录用户时用户名、密码和私钥由后端从登录用户填入。
  const account = f.auth === "account";
  const common = {
    id,
    name: f.name.trim(),
    group: f.group.trim(),
    color: f.color,
    note: f.note.trim(),
    production: f.production,
    host: f.host.trim(),
    port: Number(f.port),
    accountId: account ? f.accountId : "",
    user: f.user.trim(),
    jumpId: f.jumpId,
  };
  if (f.kind === "rdp") {
    return {
      ...common,
      kind: "rdp",
      resolution: f.resolution,
      // 跟随窗口时宽高不用，填不合法的值也照样保存默认值。
      width: isSize(f.width) ? Number(f.width) : DEFAULT_WIDTH,
      height: isSize(f.height) ? Number(f.height) : DEFAULT_HEIGHT,
      clipboard: f.clipboard,
      password: account ? "" : f.password,
    };
  }
  return {
    ...common,
    kind: "ssh",
    auth: f.auth === "account" ? "password" : f.auth,
    keyId: !account && f.auth === "key" ? f.keyId : "",
    password: !account && f.auth === "password" ? f.password : "",
    keepaliveSeconds: Number(f.keepaliveSeconds),
    initDir: f.initDir.trim(),
    sftpSudo: f.sftpSudo,
    incognito: f.incognito,
  };
}

/** 密码输入框：已保存的密码显示为“已保存”，留空表示不修改。 */
function SecretField({
  label,
  saved,
  value,
  hint,
  onChange,
  onKeyDown,
  className,
}: {
  label: string;
  saved: boolean;
  value: string;
  hint?: string;
  onChange: (v: string) => void;
  onKeyDown?: (e: KeyboardEvent<HTMLInputElement>) => void;
  className?: string;
}) {
  const { t } = useI18n();
  return (
    <Field label={label} hint={saved ? t("editor.secretSaved") : hint} className={className}>
      <PasswordInput
        value={value}
        placeholder={saved ? "••••••••" : undefined}
        onChange={(e) => onChange(e.target.value)}
        onKeyDown={onKeyDown}
      />
    </Field>
  );
}

export default function ConnEditor({
  open,
  conn,
  group = "",
  onClose,
}: {
  open: boolean;
  /** 为 null 时新建。 */
  conn: ConnView | null;
  /** 新建时预填的分组。 */
  group?: string;
  onClose: () => void;
}) {
  const { t } = useI18n();
  const toast = useToast();
  const { conns } = useConns();
  const { settings } = useSettings();
  const [form, setForm] = useState<Form>(() => toForm(conn));
  const [tab, setTab] = useState<EditorTab>("general");
  const [errors, setErrors] = useState<Errors>({});
  const [busy, setBusy] = useState<"save" | "test" | null>(null);
  const { keys } = useKeys();
  const { accounts } = useAccounts();
  const [manage, setManage] = useState<KeysTab | null>(null);
  const [quickNewAccount, setQuickNewAccount] = useState(false);

  useEffect(() => {
    if (!open) return;
    const next = toForm(conn);
    if (!conn && group) next.group = group;
    setForm(next);
    setTab("general");
    setErrors({});
  }, [open, conn, group]);

  const f = form;
  const set = <K extends keyof Form>(k: K, v: Form[K]) => {
    setForm((x) => ({ ...x, [k]: v }));
    if (errors[k]) setErrors((e) => ({ ...e, [k]: undefined }));
  };

  // 切换类型时端口、用户名还是原类型的默认值就换成新类型的默认值；选的登录用户不能用于新类型时取消选择。
  const setKind = (kind: ConnKind) => {
    setForm((x) => {
      const acc = accounts.find((a) => a.id === x.accountId);
      const disallowAcc = acc && !accountAllows(acc, kind);
      return {
        ...x,
        kind,
        port: x.port.trim() === String(DEFAULT_PORT[x.kind]) ? String(DEFAULT_PORT[kind]) : x.port,
        user:
          x.user.trim() === DEFAULT_USER[x.kind] || (disallowAcc && x.user === acc.user)
            ? DEFAULT_USER[kind]
            : x.user,
        auth: kind === "rdp" && x.auth !== "account" ? "password" : x.auth,
        accountId: disallowAcc ? "" : x.accountId,
      };
    });
    setErrors({});
  };

  const setAuth = (auth: AuthMode) => {
    setForm((x) => {
      const acc = auth === "account" ? accounts.find((a) => a.id === x.accountId) : undefined;
      return { ...x, auth, user: acc?.user ?? x.user };
    });
    setErrors((e) => ({ ...e, user: undefined, accountId: undefined, keyId: undefined }));
  };

  // 刚新建的用户还不在列表里，由调用方直接传入。
  const setAccount = (id: string, acc = accounts.find((a) => a.id === id)) => {
    setForm((x) => ({ ...x, auth: "account", accountId: id, user: acc?.user ?? x.user }));
    setErrors((e) => ({ ...e, user: undefined, accountId: undefined, keyId: undefined }));
  };

  const rdp = f.kind === "rdp";
  const useAccount = f.auth === "account";
  const usableAccounts = useMemo(() => accounts.filter((a) => accountAllows(a, f.kind)), [accounts, f.kind]);
  const account = usableAccounts.find((a) => a.id === f.accountId);

  const groups = useMemo(() => [...new Set([...loadGroups(), ...conns.map((c) => c.group).filter(Boolean)])] as string[], [conns, open]);

  // 可选的跳板机：只能是 SSH 连接；编辑时排除自己和跳板链上经过自己的连接，避免成环。
  const jumpOptions = useMemo(() => {
    const byId = new Map(conns.map((c) => [c.id, c]));
    const passesSelf = (id: string) => {
      const seen = new Set<string>();
      for (let cur: string | undefined = id; cur && !seen.has(cur); ) {
        if (cur === conn?.id) return true;
        seen.add(cur);
        cur = byId.get(cur)?.jumpId;
      }
      return false;
    };
    return conns.filter((c) => c.kind === "ssh" && !(conn && passesSelf(c.id)));
  }, [conns, conn]);

  const keyOptions = useMemo<SelectOption[]>(() => {
    const opts: SelectOption[] = keys.map((k) => ({ value: k.id, label: k.name, sub: keyTypeLabel(k) }));
    // 选中的密钥已被删除时也显示出来，保存时由后端报错。
    if (f.keyId && !keys.some((k) => k.id === f.keyId)) opts.push({ value: f.keyId, label: t("editor.keyMissing") });
    return [{ value: "", label: t(keys.length ? "editor.keyNone" : "editor.keyEmpty") }, ...opts];
  }, [keys, f.keyId, t]);

  // 只列出能用于当前类型的登录用户。
  const accountOptions = useMemo<SelectOption[]>(() => {
    const opts: SelectOption[] = usableAccounts.map((a) => {
      const authLabel = a.auth === "key" ? keys.find((k) => k.id === a.keyId)?.name ?? t("editor.auth.key") : t("editor.auth.password");
      return {
        value: a.id,
        label: a.name,
        sub: a.name === a.user ? authLabel : `@${a.user} · ${authLabel}`,
      };
    });
    // 选中的用户已被删除或不能用于这种连接时也显示出来，保存时由后端报错。
    if (f.accountId && !opts.some((o) => o.value === f.accountId)) opts.push({ value: f.accountId, label: t("editor.accountMissing") });
    return [{ value: "", label: t(usableAccounts.length ? "editor.accountNone" : "editor.accountEmpty") }, ...opts];
  }, [usableAccounts, keys, f.accountId, t]);

  const accountHint = (a: AccountView) =>
    t("editor.accountHint", {
      user: a.user,
      auth: a.auth === "key" ? `${t("editor.auth.key")} ${keys.find((k) => k.id === a.keyId)?.name ?? ""}`.trim() : t("editor.auth.password"),
    });

  const hasGeneralError = Boolean(errors.host || errors.port || errors.accountId || errors.user || errors.keyId);
  const hasAdvancedError = Boolean(errors.keepaliveSeconds || errors.width || errors.height);
  const hasCustomAdvanced =
    Boolean(f.jumpId) ||
    f.production ||
    (rdp
      ? f.resolution === "fixed" || !f.clipboard
      : f.incognito || Boolean(f.initDir.trim()) || f.sftpSudo !== "default" || f.keepaliveSeconds.trim() !== "30");

  const check = () => {
    const errs = validate(f);
    setErrors(errs);
    const genErr = Boolean(errs.host || errs.port || errs.accountId || errs.user || errs.keyId);
    const advErr = Boolean(errs.keepaliveSeconds || errs.width || errs.height);
    if (genErr && tab !== "general") setTab("general");
    else if (!genErr && advErr && tab !== "advanced") setTab("advanced");
    return Object.keys(errs).length === 0;
  };

  const submit = async () => {
    if (busy || !check()) return;
    setBusy("save");
    try {
      await connApi.save(toReq(f, conn?.id ?? ""));
      toast.success(t(conn ? "editor.saved" : "editor.created"));
      onClose();
    } catch (e) {
      toast.error(e);
    } finally {
      setBusy(null);
    }
  };

  const test = async () => {
    if (busy || !check()) return;
    setBusy("test");
    try {
      const r = await connApi.test(toReq(f, conn?.id ?? ""));
      toast.success(
        r.serverVersion ? t("editor.testOkVersion", { ms: r.elapsedMs, version: r.serverVersion }) : t("editor.testOk", { ms: r.elapsedMs }),
      );
    } catch (e) {
      toast.error(e);
    } finally {
      setBusy(null);
    }
  };

  const onEnter = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter") submit();
  };

  return (
    <Dialog
      open={open}
      onClose={busy ? undefined : onClose}
      divided
      bodyClassName="px-5 py-3.5"
      title={
        <span className="flex items-center gap-2">
          <KindIcon kind={f.kind} />
          <span className="truncate">{t(conn ? "editor.titleEdit" : "editor.titleNew", { kind: KIND_LABEL[f.kind] })}</span>
          {conn?.name && <span className="truncate text-xs font-normal text-muted">· {conn.name}</span>}
        </span>
      }
      width="max-w-xl"
      footer={
        <>
          <Button
            icon={busy === "test" ? <Loader2 size={14} className="animate-spin" /> : <PlugZap size={14} />}
            onClick={test}
            disabled={!!busy}
          >
            {t(busy === "test" ? "editor.testing" : "editor.test")}
          </Button>
          <span className="flex-1" />
          <Button onClick={onClose} disabled={!!busy}>
            {t("common.cancel")}
          </Button>
          <Button
            variant="primary"
            icon={busy === "save" ? <Loader2 size={14} className="animate-spin" /> : undefined}
            onClick={submit}
            disabled={!!busy}
          >
            {t("common.save")}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-3">
        {!conn && (
          <div className="grid grid-cols-2 gap-2.5">
            {(["ssh", "rdp"] as const).map((k) => {
              const active = f.kind === k;
              return (
                <button
                  key={k}
                  type="button"
                  onClick={() => setKind(k)}
                  className={cx(
                    "flex items-center gap-3 rounded-xl border p-2.5 text-left transition-all",
                    active ? "border-accent bg-accent/6 ring-1 ring-accent/25" : "border-line bg-surface hover:bg-subtle/60",
                  )}
                >
                  <KindIcon kind={k} className="!size-8 !rounded-lg" />
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-1.5">
                      <span className="truncate text-sm font-medium">{t(`editor.kind.${k}`)}</span>
                    </div>
                    <div className="truncate text-[11px] text-muted">{t(`editor.kindDesc.${k}`)}</div>
                  </div>
                  <span className="flex size-4 shrink-0 items-center justify-center text-accent">{active && <Check size={15} />}</span>
                </button>
              );
            })}
          </div>
        )}

        <Segmented<EditorTab>
          value={tab}
          onChange={setTab}
          className="flex w-full [&>button]:flex-1 [&>button]:justify-center"
          options={[
            {
              value: "general",
              label: (
                <>
                  <Settings2 size={14} />
                  <span>{t("editor.tab.general")}</span>
                  {hasGeneralError && <span className="size-1.5 rounded-full bg-red-500" />}
                </>
              ),
            },
            {
              value: "advanced",
              label: (
                <>
                  <SlidersHorizontal size={14} />
                  <span>{t("editor.section.advanced")}</span>
                  {hasAdvancedError ? (
                    <span className="size-1.5 rounded-full bg-red-500" />
                  ) : f.production ? (
                    <span className="size-1.5 rounded-full bg-red-500" />
                  ) : hasCustomAdvanced ? (
                    <span className="size-1.5 rounded-full bg-accent" />
                  ) : null}
                </>
              ),
            },
          ]}
        />

        <div className="flex min-h-78 flex-col gap-3">
          {tab === "general" ? (
            <>
              <div className="flex flex-col gap-3 rounded-xl border border-line bg-subtle/30 p-3.5">
                <div className="flex items-start gap-3">
                  <Field label={t("editor.host")} error={errors.host} className="min-w-0 flex-1">
                    <Input
                      autoFocus={!conn}
                      mono
                      placeholder="example.com"
                      value={f.host}
                      invalid={!!errors.host}
                      onChange={(e) => set("host", e.target.value)}
                      onKeyDown={onEnter}
                    />
                  </Field>
                  <Field label={t("editor.port")} error={errors.port} className="w-24 shrink-0">
                    <Input
                      mono
                      value={f.port}
                      invalid={!!errors.port}
                      onChange={(e) => set("port", e.target.value)}
                      onKeyDown={onEnter}
                    />
                  </Field>
                </div>

                <Field label={t("editor.authType")}>
                  <Segmented<AuthMode>
                    value={f.auth}
                    onChange={setAuth}
                    className="flex w-full [&>button]:flex-1 [&>button]:justify-center"
                    options={
                      rdp
                        ? [
                            {
                              value: "account",
                              label: (
                                <>
                                  <UserRound size={13} />
                                  {t("editor.account")}
                                </>
                              ),
                            },
                            {
                              value: "password",
                              label: (
                                <>
                                  <Lock size={13} />
                                  {t("editor.auth.password")}
                                </>
                              ),
                            },
                          ]
                        : [
                            {
                              value: "account",
                              label: (
                                <>
                                  <UserRound size={13} />
                                  {t("editor.account")}
                                </>
                              ),
                            },
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
                          ]
                    }
                  />
                </Field>

                {useAccount ? (
                  <Field label={t("editor.account")} error={errors.accountId}>
                    <div className="flex gap-2">
                      <Select
                        className="min-w-0 flex-1"
                        value={f.accountId}
                        invalid={!!errors.accountId}
                        onChange={(v) => setAccount(v)}
                        options={accountOptions}
                      />
                      <Button
                        type="button"
                        icon={<Plus size={14} />}
                        onClick={() => (usableAccounts.length ? setManage("accounts") : setQuickNewAccount(true))}
                      >
                        {t(usableAccounts.length ? "editor.manageAccounts" : "accounts.newTitle")}
                      </Button>
                    </div>
                    {account ? (
                      <div className="mt-0.5 flex items-center gap-1.5 rounded-lg border border-line/70 bg-surface px-2.5 py-1.5 text-xs text-muted">
                        <UserRound size={13} className="shrink-0 text-accent" />
                        <span className="truncate">{accountHint(account)}</span>
                      </div>
                    ) : (
                      <span className="text-xs text-muted">{t("editor.accountNoneHint")}</span>
                    )}
                  </Field>
                ) : f.auth === "password" ? (
                  <div className="flex items-start gap-3">
                    <Field
                      label={t("editor.user")}
                      error={errors.user}
                      hint={rdp ? t("editor.rdpUserHint") : undefined}
                      className="min-w-0 flex-1"
                    >
                      <Input
                        value={f.user}
                        invalid={!!errors.user}
                        onChange={(e) => set("user", e.target.value)}
                        onKeyDown={onEnter}
                      />
                    </Field>
                    <SecretField
                      label={t("editor.password")}
                      saved={!!conn?.hasPassword}
                      value={f.password}
                      onChange={(v) => set("password", v)}
                      onKeyDown={onEnter}
                      className="min-w-0 flex-1"
                    />
                  </div>
                ) : (
                  <div className="flex items-start gap-3">
                    <Field label={t("editor.user")} error={errors.user} className="w-40 shrink-0">
                      <Input
                        value={f.user}
                        invalid={!!errors.user}
                        onChange={(e) => set("user", e.target.value)}
                        onKeyDown={onEnter}
                      />
                    </Field>
                    <Field label={t("editor.key")} error={errors.keyId} hint={t("editor.keyHint")} className="min-w-0 flex-1">
                      <div className="flex gap-2">
                        <Select
                          className="min-w-0 flex-1"
                          value={f.keyId}
                          invalid={!!errors.keyId}
                          onChange={(v) => set("keyId", v)}
                          options={keyOptions}
                        />
                        <Button type="button" icon={<Plus size={14} />} onClick={() => setManage("keys")}>
                          {t(keys.length ? "editor.manageKeys" : "editor.keyAdd")}
                        </Button>
                      </div>
                    </Field>
                  </div>
                )}
              </div>

              <div className="flex flex-col gap-3 rounded-xl border border-line bg-subtle/30 p-3.5">
                <div className="flex items-start gap-3">
                  <Field label={t("editor.name")} className="min-w-0 flex-1">
                    <Input
                      placeholder={t("editor.namePlaceholder")}
                      value={f.name}
                      onChange={(e) => set("name", e.target.value)}
                      onKeyDown={onEnter}
                    />
                  </Field>
                  <Field label={t("editor.group")} className="min-w-0 flex-1">
                    <ComboInput
                      options={groups}
                      placeholder={t("editor.groupPlaceholder")}
                      emptyLabel={t("editor.groupPlaceholder")}
                      value={f.group}
                      onChange={(v) => set("group", v)}
                    />
                  </Field>
                </div>

                <div className="flex items-center justify-between gap-3">
                  <span className="text-xs font-medium text-muted">{t("editor.color")}</span>
                  <div className="flex items-center gap-2 py-0.5">
                    <button
                      type="button"
                      title={t("editor.colorNone")}
                      onClick={() => set("color", "")}
                      className={cx(
                        "flex size-5 items-center justify-center rounded-full border border-dashed border-muted/60 transition-transform hover:scale-110",
                        !f.color && "ring-2 ring-accent ring-offset-2 ring-offset-surface",
                      )}
                    />
                    {(Object.keys(CONN_COLORS) as ConnColor[]).map((c) => {
                      const active = f.color === c;
                      return (
                        <button
                          key={c}
                          type="button"
                          onClick={() => set("color", c)}
                          className={cx(
                            "flex size-5 items-center justify-center rounded-full text-white transition-transform hover:scale-110",
                            CONN_COLORS[c],
                            active && "ring-2 ring-accent ring-offset-2 ring-offset-surface",
                          )}
                        >
                          {active && <Check size={11} strokeWidth={3} />}
                        </button>
                      );
                    })}
                  </div>
                </div>

                <Field label={t("editor.note")}>
                  <textarea
                    rows={2}
                    spellCheck={false}
                    value={f.note}
                    onChange={(e) => set("note", e.target.value)}
                    className="w-full resize-none rounded-lg border border-line bg-surface px-2.5 py-1.5 text-sm outline-none transition-colors focus:border-accent focus:ring-3 focus:ring-accent/15"
                  />
                </Field>
              </div>
            </>
          ) : (
            <>
              <div className="flex flex-col gap-3.5 rounded-xl border border-line bg-subtle/30 p-3.5">
                <Field label={t("editor.jump")} hint={jumpOptions.length ? t(rdp ? "editor.jumpHintRdp" : "editor.jumpHint") : undefined}>
                  <Select
                    value={f.jumpId}
                    onChange={(v) => set("jumpId", v)}
                    options={[
                      { value: "", label: t("editor.jumpNone") },
                      ...jumpOptions.map((c) => ({ value: c.id, label: c.name, sub: connAddress(c), icon: <Waypoints size={13} /> })),
                      // 引用的连接已被删除时也显示出来，保存时由后端报错。
                      ...(f.jumpId && !jumpOptions.some((c) => c.id === f.jumpId) ? [{ value: f.jumpId, label: f.jumpId }] : []),
                    ]}
                  />
                </Field>

                {rdp ? (
                  <>
                    <div className="flex items-center justify-between gap-3">
                      <span className="text-xs font-medium text-muted">{t("editor.resolution")}</span>
                      <Segmented<RdpResolution>
                        value={f.resolution}
                        onChange={(v) => set("resolution", v)}
                        options={[
                          { value: "fit", label: t("editor.resolution.fit") },
                          { value: "fixed", label: t("editor.resolution.fixed") },
                        ]}
                      />
                    </div>
                    {f.resolution === "fixed" && (
                      <div className="flex flex-col gap-2 rounded-lg bg-subtle/45 p-2.5">
                        <div className="flex items-start gap-2">
                          <Field label={t("editor.width")} error={errors.width} className="min-w-0 flex-1">
                            <Input
                              mono
                              value={f.width}
                              invalid={!!errors.width}
                              onChange={(e) => set("width", e.target.value)}
                              onKeyDown={onEnter}
                            />
                          </Field>
                          <span className="mt-7 text-muted">×</span>
                          <Field label={t("editor.height")} error={errors.height} className="min-w-0 flex-1">
                            <Input
                              mono
                              value={f.height}
                              invalid={!!errors.height}
                              onChange={(e) => set("height", e.target.value)}
                              onKeyDown={onEnter}
                            />
                          </Field>
                        </div>
                        <div className="flex flex-wrap items-center gap-1.5">
                          {RDP_PRESETS.map(([w, h]) => {
                            const active = f.width.trim() === String(w) && f.height.trim() === String(h);
                            return (
                              <button
                                key={`${w}x${h}`}
                                type="button"
                                onClick={() => {
                                  set("width", String(w));
                                  set("height", String(h));
                                }}
                                className={cx(
                                  "rounded-md border px-2 py-0.5 font-mono text-[11px] transition-colors",
                                  active
                                    ? "border-accent bg-accent/10 font-medium text-accent"
                                    : "border-line bg-surface text-muted hover:text-ink",
                                )}
                              >
                                {w} × {h}
                              </button>
                            );
                          })}
                        </div>
                      </div>
                    )}
                  </>
                ) : (
                  <>
                    <div className="flex items-start gap-3">
                      <Field label={t("editor.initDir")} className="min-w-0 flex-1">
                        <Input
                          mono
                          placeholder={t("editor.initDirPlaceholder")}
                          value={f.initDir}
                          onChange={(e) => set("initDir", e.target.value)}
                          onKeyDown={onEnter}
                        />
                      </Field>
                      <Field
                        label={t("editor.keepalive")}
                        error={errors.keepaliveSeconds}
                        hint={t("editor.keepaliveHint")}
                        className="w-40 shrink-0"
                      >
                        <Input
                          mono
                          value={f.keepaliveSeconds}
                          invalid={!!errors.keepaliveSeconds}
                          onChange={(e) => set("keepaliveSeconds", e.target.value)}
                          onKeyDown={onEnter}
                        />
                      </Field>
                    </div>

                    <Field
                      label={t("editor.sftpSudo")}
                      hint={t(f.sftpSudo === "default" ? "editor.sftpSudoHintDefault" : "editor.sftpSudoHint", {
                        state: t(settings?.tools.sftp.sudo ? "editor.sftpSudo.on" : "editor.sftpSudo.off"),
                      })}
                    >
                      <Segmented<SftpSudo>
                        value={f.sftpSudo}
                        onChange={(v) => set("sftpSudo", v)}
                        className="flex w-full [&>button]:flex-1 [&>button]:justify-center"
                        options={[
                          { value: "default", label: t("editor.sftpSudo.default") },
                          { value: "off", label: t("editor.sftpSudo.off") },
                          { value: "on", label: t("editor.sftpSudo.on") },
                        ]}
                      />
                    </Field>
                  </>
                )}
              </div>

              <Card className="divide-y divide-line [&>*]:px-4 [&>*]:py-3">
                <SwitchRow
                  title={t("editor.production")}
                  badge={f.production ? <ProdBadge /> : undefined}
                  desc={t(rdp ? "editor.productionDescRdp" : "editor.productionDesc")}
                  checked={f.production}
                  onChange={(v) => set("production", v)}
                />
                {rdp ? (
                  <SwitchRow
                    title={t("editor.clipboard")}
                    desc={t("editor.clipboardDesc")}
                    checked={f.clipboard}
                    onChange={(v) => set("clipboard", v)}
                  />
                ) : (
                  <SwitchRow
                    title={t("editor.incognito")}
                    badge={f.incognito ? <IncognitoBadge /> : undefined}
                    desc={t("editor.incognitoDesc")}
                    checked={f.incognito}
                    onChange={(v) => set("incognito", v)}
                  />
                )}
              </Card>
            </>
          )}
        </div>
      </div>

      <KeysDialog
        open={!!manage}
        tab={manage ?? "keys"}
        onClose={() => setManage(null)}
        onAdded={(k) => set("keyId", k.id)}
        onAccountAdded={(a) => accountAllows(a, f.kind) && setAccount(a.id, a)}
      />
      <AccountEditor
        open={quickNewAccount}
        account={null}
        onClose={() => setQuickNewAccount(false)}
        onDone={(a) => accountAllows(a, f.kind) && setAccount(a.id, a)}
      />
    </Dialog>
  );
}
