import { PlugZap, Plus } from "lucide-react";
import { useEffect, useMemo, useState, type ReactNode } from "react";
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
import { accountAllows, accountLabel } from "./AccountsPanel";
import KeysDialog, { type KeysTab } from "./KeysDialog";
import KindIcon from "./KindIcon";
import { Button, cx, Dialog, Field, Input, PasswordInput, Segmented, Select, SwitchRow } from "./ui";

type AuthMode = AccountAuth | "account";

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

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="mt-5">
      <h3 className="mb-2.5 text-xs font-semibold tracking-wide text-muted">{title}</h3>
      <div className="flex flex-col gap-3">{children}</div>
    </section>
  );
}

const Row = ({ children }: { children: ReactNode }) => <div className="flex gap-3 [&>*]:flex-1">{children}</div>;

/** 密码输入框：已保存的密码显示为“已保存”，留空表示不修改。 */
function SecretField({
  label,
  saved,
  value,
  hint,
  onChange,
}: {
  label: string;
  saved: boolean;
  value: string;
  hint?: string;
  onChange: (v: string) => void;
}) {
  const { t } = useI18n();
  return (
    <Field label={label} hint={saved ? t("editor.secretSaved") : hint}>
      <PasswordInput
        value={value}
        placeholder={saved ? "••••••••" : undefined}
        onChange={(e) => onChange(e.target.value)}
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
  const [errors, setErrors] = useState<Errors>({});
  const [busy, setBusy] = useState<"save" | "test" | null>(null);
  const { keys } = useKeys();
  const { accounts } = useAccounts();
  const [manage, setManage] = useState<KeysTab | null>(null);

  useEffect(() => {
    if (!open) return;
    const next = toForm(conn);
    if (!conn && group) next.group = group;
    setForm(next);
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
  const account = accounts.find((a) => a.id === f.accountId && accountAllows(a, f.kind));

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

  const keyOptions = useMemo(() => {
    const opts = keys.map((k) => ({ value: k.id, label: `${k.name} · ${keyTypeLabel(k)}` }));
    // 选中的密钥已被删除时也显示出来，保存时由后端报错。
    if (f.keyId && !keys.some((k) => k.id === f.keyId)) opts.push({ value: f.keyId, label: t("editor.keyMissing") });
    return [{ value: "", label: t(keys.length ? "editor.keyNone" : "editor.keyEmpty") }, ...opts];
  }, [keys, f.keyId, t]);

  // 只列出能用于当前类型的登录用户。
  const accountOptions = useMemo(() => {
    const usable = accounts.filter((a) => accountAllows(a, f.kind));
    const opts = usable.map((a) => ({ value: a.id, label: accountLabel(a) }));
    // 选中的用户已被删除或不能用于这种连接时也显示出来，保存时由后端报错。
    if (f.accountId && !opts.some((o) => o.value === f.accountId)) opts.push({ value: f.accountId, label: t("editor.accountMissing") });
    return [{ value: "", label: t(usable.length ? "editor.accountNone" : "editor.accountEmpty") }, ...opts];
  }, [accounts, f.kind, f.accountId, t]);

  const accountHint = (a: AccountView) =>
    t("editor.accountHint", {
      user: a.user,
      auth: a.auth === "key" ? `${t("editor.auth.key")} ${keys.find((k) => k.id === a.keyId)?.name ?? ""}`.trim() : t("editor.auth.password"),
    });

  const check = () => {
    const errs = validate(f);
    setErrors(errs);
    return Object.keys(errs).length === 0;
  };

  const submit = async () => {
    if (!check()) return;
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
    if (!check()) return;
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

  return (
    <Dialog
      open={open}
      onClose={busy ? undefined : onClose}
      title={
        <span className="flex items-center gap-2">
          <KindIcon kind={f.kind} />
          {t(conn ? "editor.titleEdit" : "editor.titleNew", { kind: KIND_LABEL[f.kind] })}
        </span>
      }
      width="max-w-xl"
      footer={
        <>
          <Button icon={<PlugZap size={14} />} onClick={test} disabled={!!busy}>
            {t("editor.test")}
          </Button>
          <span className="flex-1" />
          <Button onClick={onClose} disabled={!!busy}>
            {t("common.cancel")}
          </Button>
          <Button variant="primary" onClick={submit} disabled={!!busy}>
            {t("common.save")}
          </Button>
        </>
      }
    >
      {!conn && (
        <Field label={t("editor.kind")} className="mb-3">
          <Segmented<ConnKind>
            value={f.kind}
            onChange={setKind}
            options={[
              { value: "ssh", label: t("editor.kind.ssh") },
              { value: "rdp", label: t("editor.kind.rdp") },
            ]}
          />
        </Field>
      )}
      <Row>
        <Field label={t("editor.name")}>
          <Input placeholder={t("editor.namePlaceholder")} value={f.name} onChange={(e) => set("name", e.target.value)} />
        </Field>
        <Field label={t("editor.group")}>
          <Input list="conn-groups" placeholder={t("editor.groupPlaceholder")} value={f.group} onChange={(e) => set("group", e.target.value)} />
          <datalist id="conn-groups">
            {groups.map((g) => (
              <option key={g} value={g} />
            ))}
          </datalist>
        </Field>
      </Row>

      <Section title={t("editor.section.server")}>
        <Row>
          <Field label={t("editor.host")} error={errors.host}>
            <Input placeholder="example.com" value={f.host} invalid={!!errors.host} onChange={(e) => set("host", e.target.value)} />
          </Field>
          <Field label={t("editor.port")} error={errors.port} className="!flex-none w-28">
            <Input value={f.port} invalid={!!errors.port} onChange={(e) => set("port", e.target.value)} />
          </Field>
        </Row>
        <Row>
          <Field
            label={t("editor.user")}
            error={useAccount ? errors.accountId : errors.user}
            hint={!useAccount && rdp ? t("editor.rdpUserHint") : undefined}
          >
            {useAccount ? (
              <div className="flex gap-1.5">
                <Select
                  className="min-w-0 flex-1"
                  value={f.accountId}
                  invalid={!!errors.accountId}
                  onChange={(v) => setAccount(v)}
                  options={accountOptions}
                />
                <Button
                  type="button"
                  className="w-8.5 shrink-0 px-0"
                  title={t("editor.keyAdd")}
                  aria-label={t("editor.keyAdd")}
                  onClick={() => setManage("accounts")}
                >
                  <Plus size={15} />
                </Button>
              </div>
            ) : (
              <Input value={f.user} invalid={!!errors.user} onChange={(e) => set("user", e.target.value)} />
            )}
          </Field>
          <Field label={t("editor.authType")} className="!flex-none">
            <Segmented<AuthMode>
              value={f.auth}
              onChange={setAuth}
              options={
                rdp
                  ? [
                      { value: "account", label: t("editor.account") },
                      { value: "password", label: t("editor.auth.password") },
                    ]
                  : [
                      { value: "account", label: t("editor.account") },
                      { value: "password", label: t("editor.auth.password") },
                      { value: "key", label: t("editor.auth.key") },
                    ]
              }
            />
          </Field>
        </Row>
        {useAccount ? (
          <p className="-mt-1.5 text-xs text-muted">{account ? accountHint(account) : t("editor.accountNoneHint")}</p>
        ) : (
          <>
            {f.auth === "password" && (
              <SecretField
                label={t("editor.password")}
                saved={!!conn?.hasPassword}
                value={f.password}
                onChange={(v) => set("password", v)}
              />
            )}
            {f.auth === "key" && (
              <Field label={t("editor.key")} error={errors.keyId} hint={t("editor.keyHint")}>
                <div className="flex gap-2">
                  <Select
                    className="min-w-0 flex-1"
                    value={f.keyId}
                    invalid={!!errors.keyId}
                    onChange={(v) => set("keyId", v)}
                    options={keyOptions}
                  />
                  <Button type="button" icon={<Plus size={14} />} onClick={() => setManage("keys")}>
                    {t("editor.keyAdd")}
                  </Button>
                </div>
              </Field>
            )}
          </>
        )}
      </Section>

      <Section title={t("editor.section.advanced")}>
        <Field label={t("editor.jump")} hint={jumpOptions.length ? t(rdp ? "editor.jumpHintRdp" : "editor.jumpHint") : undefined}>
          <Select
            value={f.jumpId}
            onChange={(v) => set("jumpId", v)}
            options={[
              { value: "", label: t("editor.jumpNone") },
              ...jumpOptions.map((c) => ({ value: c.id, label: `${c.name} · ${connAddress(c)}` })),
              // 引用的连接已被删除时也显示出来，保存时由后端报错。
              ...(f.jumpId && !jumpOptions.some((c) => c.id === f.jumpId) ? [{ value: f.jumpId, label: f.jumpId }] : []),
            ]}
          />
        </Field>
        {rdp ? (
          <Row>
            <Field label={t("editor.resolution")} className="!flex-none">
              <Segmented<RdpResolution>
                value={f.resolution}
                onChange={(v) => set("resolution", v)}
                options={[
                  { value: "fit", label: t("editor.resolution.fit") },
                  { value: "fixed", label: t("editor.resolution.fixed") },
                ]}
              />
            </Field>
            {f.resolution === "fixed" && (
              <>
                <Field label={t("editor.width")} error={errors.width}>
                  <Input value={f.width} invalid={!!errors.width} onChange={(e) => set("width", e.target.value)} />
                </Field>
                <Field label={t("editor.height")} error={errors.height}>
                  <Input value={f.height} invalid={!!errors.height} onChange={(e) => set("height", e.target.value)} />
                </Field>
              </>
            )}
          </Row>
        ) : (
          <Row>
            <Field label={t("editor.initDir")}>
              <Input mono placeholder={t("editor.initDirPlaceholder")} value={f.initDir} onChange={(e) => set("initDir", e.target.value)} />
            </Field>
            <Field label={t("editor.keepalive")} error={errors.keepaliveSeconds} hint={t("editor.keepaliveHint")} className="!flex-none w-36">
              <Input value={f.keepaliveSeconds} invalid={!!errors.keepaliveSeconds} onChange={(e) => set("keepaliveSeconds", e.target.value)} />
            </Field>
          </Row>
        )}
        {!rdp && (
          <Field
            label={t("editor.sftpSudo")}
            hint={t(f.sftpSudo === "default" ? "editor.sftpSudoHintDefault" : "editor.sftpSudoHint", {
              state: t(settings?.tools.sftp.sudo ? "editor.sftpSudo.on" : "editor.sftpSudo.off"),
            })}
          >
            <Segmented<SftpSudo>
              value={f.sftpSudo}
              onChange={(v) => set("sftpSudo", v)}
              options={[
                { value: "default", label: t("editor.sftpSudo.default") },
                { value: "off", label: t("editor.sftpSudo.off") },
                { value: "on", label: t("editor.sftpSudo.on") },
              ]}
            />
          </Field>
        )}
      </Section>

      <Section title={t("editor.section.other")}>
        <SwitchRow
          title={t("editor.production")}
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
            desc={t("editor.incognitoDesc")}
            checked={f.incognito}
            onChange={(v) => set("incognito", v)}
          />
        )}
        <div className="flex flex-col gap-1">
          <span className="text-xs font-medium text-muted">{t("editor.color")}</span>
          <div className="flex items-center gap-2 py-1">
            <button
              type="button"
              title={t("editor.colorNone")}
              onClick={() => set("color", "")}
              className={cx(
                "size-5 rounded-full border border-dashed border-muted/60",
                !f.color && "ring-2 ring-accent ring-offset-2 ring-offset-surface",
              )}
            />
            {(Object.keys(CONN_COLORS) as ConnColor[]).map((c) => (
              <button
                key={c}
                type="button"
                onClick={() => set("color", c)}
                className={cx("size-5 rounded-full", CONN_COLORS[c], f.color === c && "ring-2 ring-accent ring-offset-2 ring-offset-surface")}
              />
            ))}
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
      </Section>
      <div className="h-2" />
      <KeysDialog
        open={!!manage}
        tab={manage ?? "keys"}
        onClose={() => setManage(null)}
        onAdded={(k) => set("keyId", k.id)}
        onAccountAdded={(a) => accountAllows(a, f.kind) && setAccount(a.id, a)}
      />
    </Dialog>
  );
}
