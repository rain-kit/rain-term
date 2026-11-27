import { FolderOpen, PlugZap } from "lucide-react";
import { useEffect, useMemo, useState, type ReactNode } from "react";
import { connApi } from "../lib/api";
import { loadGroups } from "../lib/groups";
import { CONN_COLORS, connAddress, DEFAULT_PORT, KIND_LABEL, RDP_MAX_SIZE, RDP_MIN_SIZE, type ConnColor } from "../lib/format";
import { t, useI18n } from "../lib/i18n";
import { useConns, useToast } from "../lib/store";
import type { AuthType, ConnKind, ConnView, RdpResolution, SaveConnReq } from "../lib/types";
import KindIcon from "./KindIcon";
import { Button, cx, Dialog, Field, IconButton, Input, PasswordInput, Segmented, Select, SwitchRow } from "./ui";

interface Form {
  kind: ConnKind;
  name: string;
  group: string;
  color: string;
  note: string;
  production: boolean;
  host: string;
  port: string;
  user: string;
  password: string;
  jumpId: string;
  // SSH
  incognito: boolean;
  auth: AuthType;
  keyPath: string;
  keyPassphrase: string;
  keepaliveSeconds: string;
  initDir: string;
  // 远程桌面
  domain: string;
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
    user: c?.user ?? "",
    password: "",
    jumpId: c?.jumpId ?? "",
    incognito: !!ssh?.incognito,
    auth: ssh?.auth ?? "password",
    keyPath: ssh?.keyPath ?? "",
    keyPassphrase: "",
    keepaliveSeconds: String(ssh?.keepaliveSeconds ?? 30),
    initDir: ssh?.initDir ?? "",
    domain: rdp?.domain ?? "",
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
  if (!f.user.trim()) e.user = t("editor.required");
  if (f.kind === "ssh") {
    if (f.auth === "key" && !f.keyPath.trim()) e.keyPath = t("editor.err.keyPath");
    if (!isInt(f.keepaliveSeconds, 0, 3600)) e.keepaliveSeconds = t("editor.err.keepalive");
  } else if (f.resolution === "fixed") {
    const msg = t("editor.err.rdpSize", { min: RDP_MIN_SIZE, max: RDP_MAX_SIZE });
    if (!isSize(f.width)) e.width = msg;
    if (!isSize(f.height)) e.height = msg;
  }
  return e;
}

function toReq(f: Form, id: string, clear: { password: boolean; keyPassphrase: boolean }): SaveConnReq {
  const common = {
    id,
    name: f.name.trim(),
    group: f.group.trim(),
    color: f.color,
    note: f.note.trim(),
    production: f.production,
    host: f.host.trim(),
    port: Number(f.port),
    user: f.user.trim(),
    jumpId: f.jumpId,
    clearPassword: clear.password,
  };
  if (f.kind === "rdp") {
    return {
      ...common,
      kind: "rdp",
      domain: f.domain.trim(),
      resolution: f.resolution,
      // 跟随窗口时宽高不用，填不合法的值也照样保存默认值。
      width: isSize(f.width) ? Number(f.width) : DEFAULT_WIDTH,
      height: isSize(f.height) ? Number(f.height) : DEFAULT_HEIGHT,
      clipboard: f.clipboard,
      password: f.password,
    };
  }
  return {
    ...common,
    kind: "ssh",
    auth: f.auth,
    keyPath: f.auth === "key" ? f.keyPath.trim() : "",
    password: f.auth === "password" ? f.password : "",
    keyPassphrase: f.auth === "key" ? f.keyPassphrase : "",
    keepaliveSeconds: Number(f.keepaliveSeconds),
    initDir: f.initDir.trim(),
    incognito: f.incognito,
    clearKeyPassphrase: clear.keyPassphrase,
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

/** 密码输入框：已保存的密码显示为“已保存”，留空表示不修改，也可以标记为保存时删除。 */
function SecretField({
  label,
  saved,
  cleared,
  value,
  hint,
  revealing,
  revealed,
  onChange,
  onClear,
  onReveal,
}: {
  label: string;
  saved: boolean;
  cleared: boolean;
  value: string;
  hint?: string;
  revealing?: boolean;
  revealed?: boolean;
  onChange: (v: string) => void;
  onClear: (v: boolean) => void;
  /** 不传时不能查看保存的密码。 */
  onReveal?: () => void;
}) {
  const { t } = useI18n();
  const link = "ml-1.5 text-accent hover:underline disabled:opacity-50 disabled:no-underline";
  const savedHint = saved ? (
    cleared ? (
      <>
        {t("editor.secretWillClear")}
        <button type="button" className={link} onClick={() => onClear(false)}>
          {t("editor.secretUndo")}
        </button>
      </>
    ) : (
      <>
        {t("editor.secretSaved")}
        {onReveal && !value && (
          <button type="button" className={link} disabled={revealing} onClick={onReveal}>
            {t(revealing ? "editor.secretRevealing" : "editor.secretReveal")}
          </button>
        )}
        <button type="button" className={link} onClick={() => onClear(true)}>
          {t("editor.secretClear")}
        </button>
      </>
    )
  ) : null;
  return (
    <Field label={label} hint={savedHint ?? hint}>
      <PasswordInput
        revealed={revealed}
        value={value}
        placeholder={saved && !cleared ? "••••••••" : undefined}
        onChange={(e) => {
          onChange(e.target.value);
          if (e.target.value) onClear(false);
        }}
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
  const [form, setForm] = useState<Form>(() => toForm(conn));
  const [errors, setErrors] = useState<Errors>({});
  const [clear, setClear] = useState({ password: false, keyPassphrase: false });
  const [busy, setBusy] = useState<"save" | "test" | null>(null);
  const [revealing, setRevealing] = useState(false);
  const [revealed, setRevealed] = useState(false);

  useEffect(() => {
    if (!open) return;
    const next = toForm(conn);
    if (!conn && group) next.group = group;
    setForm(next);
    setErrors({});
    setClear({ password: false, keyPassphrase: false });
    setRevealed(false);
  }, [open, conn, group]);

  const f = form;
  const set = <K extends keyof Form>(k: K, v: Form[K]) => {
    setForm((x) => ({ ...x, [k]: v }));
    if (errors[k]) setErrors((e) => ({ ...e, [k]: undefined }));
  };
  // 切换类型时端口还是原类型的默认值就换成新类型的默认值。
  const setKind = (kind: ConnKind) => {
    setForm((x) => ({ ...x, kind, port: x.port.trim() === String(DEFAULT_PORT[x.kind]) ? String(DEFAULT_PORT[kind]) : x.port }));
    setErrors({});
  };
  const rdp = f.kind === "rdp";

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

  const pickKey = async () => {
    try {
      const p = await connApi.pickKeyFile();
      if (p) set("keyPath", p);
    } catch (e) {
      toast.error(e);
    }
  };

  // 只有按密码认证保存的连接才能查看密码，查看前要先通过电脑密码验证。
  const canReveal = !!conn?.hasPassword && (conn.kind === "rdp" || conn.auth === "password");
  const reveal = async () => {
    if (!conn) return;
    setRevealing(true);
    try {
      const password = await connApi.revealPassword(conn.id);
      if (password !== null) {
        set("password", password);
        setRevealed(true);
      }
    } catch (e) {
      toast.error(e);
    } finally {
      setRevealing(false);
    }
  };

  const check = () => {
    const errs = validate(f);
    setErrors(errs);
    return Object.keys(errs).length === 0;
  };

  const submit = async () => {
    if (!check()) return;
    setBusy("save");
    try {
      await connApi.save(toReq(f, conn?.id ?? "", clear));
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
      const r = await connApi.test(toReq(f, conn?.id ?? "", clear));
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
        {rdp ? (
          <>
            <Row>
              <Field label={t("editor.user")} error={errors.user} hint={t("editor.rdpUserHint")}>
                <Input value={f.user} invalid={!!errors.user} onChange={(e) => set("user", e.target.value)} />
              </Field>
              <Field label={t("editor.domain")} hint={t("editor.domainHint")}>
                <Input value={f.domain} onChange={(e) => set("domain", e.target.value)} />
              </Field>
            </Row>
            <SecretField
              label={t("editor.password")}
              saved={!!conn?.hasPassword}
              cleared={clear.password}
              value={f.password}
              revealing={revealing}
              revealed={revealed}
              onChange={(v) => set("password", v)}
              onClear={(v) => setClear((c) => ({ ...c, password: v }))}
              onReveal={canReveal ? reveal : undefined}
            />
          </>
        ) : (
          <>
            <Row>
              <Field label={t("editor.user")} error={errors.user}>
                <Input value={f.user} invalid={!!errors.user} onChange={(e) => set("user", e.target.value)} />
              </Field>
              <Field label={t("editor.authType")} className="!flex-none">
                <Segmented<AuthType>
                  value={f.auth}
                  onChange={(v) => set("auth", v)}
                  options={[
                    { value: "password", label: t("editor.auth.password") },
                    { value: "key", label: t("editor.auth.key") },
                    { value: "agent", label: "SSH Agent" },
                  ]}
                />
              </Field>
            </Row>
            {f.auth === "password" && (
              <SecretField
                label={t("editor.password")}
                saved={!!conn?.hasPassword}
                cleared={clear.password}
                value={f.password}
                revealing={revealing}
                revealed={revealed}
                onChange={(v) => set("password", v)}
                onClear={(v) => setClear((c) => ({ ...c, password: v }))}
                onReveal={canReveal ? reveal : undefined}
              />
            )}
            {f.auth === "key" && (
              <>
                <Field label={t("editor.keyPath")} error={errors.keyPath}>
                  <Input
                    mono
                    placeholder="~/.ssh/id_ed25519"
                    value={f.keyPath}
                    invalid={!!errors.keyPath}
                    onChange={(e) => set("keyPath", e.target.value)}
                    suffix={
                      <IconButton label={t("editor.pickFile")} size="sm" type="button" onClick={pickKey}>
                        <FolderOpen size={15} />
                      </IconButton>
                    }
                  />
                </Field>
                <SecretField
                  label={t("editor.keyPassphrase")}
                  saved={!!conn?.hasKeyPassphrase}
                  cleared={clear.keyPassphrase}
                  value={f.keyPassphrase}
                  hint={t("editor.keyPassphraseHint")}
                  onChange={(v) => set("keyPassphrase", v)}
                  onClear={(v) => setClear((c) => ({ ...c, keyPassphrase: v }))}
                />
              </>
            )}
            {f.auth === "agent" && <p className="text-xs text-muted">{t("editor.agentHint")}</p>}
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
    </Dialog>
  );
}
