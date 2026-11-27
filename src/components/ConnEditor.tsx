import { FolderOpen, PlugZap } from "lucide-react";
import { useEffect, useMemo, useState, type ReactNode } from "react";
import { connApi } from "../lib/api";
import { loadGroups } from "../lib/groups";
import { CONN_COLORS, connAddress, DEFAULT_SSH_PORT, KIND_LABEL, type ConnColor } from "../lib/format";
import { t, useI18n } from "../lib/i18n";
import { useConns, useToast } from "../lib/store";
import type { AuthType, ConnView, SaveConnReq } from "../lib/types";
import KindIcon from "./KindIcon";
import { Button, cx, Dialog, Field, IconButton, Input, PasswordInput, Segmented, Select, SwitchRow } from "./ui";

interface Form {
  name: string;
  group: string;
  color: string;
  note: string;
  production: boolean;
  incognito: boolean;
  host: string;
  port: string;
  user: string;
  password: string;
  auth: AuthType;
  keyPath: string;
  keyPassphrase: string;
  jumpId: string;
  keepaliveSeconds: string;
  initDir: string;
}

type Errors = Partial<Record<keyof Form, string>>;

function toForm(c?: ConnView | null): Form {
  return {
    name: c?.name ?? "",
    group: c?.group ?? "",
    color: c?.color ?? "",
    note: c?.note ?? "",
    production: !!c?.production,
    incognito: !!c?.incognito,
    host: c?.host ?? "",
    port: String(c?.port ?? DEFAULT_SSH_PORT),
    user: c?.user ?? "",
    password: "",
    auth: c?.auth ?? "password",
    keyPath: c?.keyPath ?? "",
    keyPassphrase: "",
    jumpId: c?.jumpId ?? "",
    keepaliveSeconds: String(c?.keepaliveSeconds ?? 30),
    initDir: c?.initDir ?? "",
  };
}

const isInt = (v: string, min: number, max: number) => /^\d+$/.test(v.trim()) && Number(v) >= min && Number(v) <= max;

function validate(f: Form): Errors {
  const e: Errors = {};
  if (!f.host.trim()) e.host = t("editor.required");
  if (!isInt(f.port, 1, 65535)) e.port = t("editor.err.port");
  if (!f.user.trim()) e.user = t("editor.required");
  if (f.auth === "key" && !f.keyPath.trim()) e.keyPath = t("editor.err.keyPath");
  if (!isInt(f.keepaliveSeconds, 0, 3600)) e.keepaliveSeconds = t("editor.err.keepalive");
  return e;
}

function toReq(f: Form, id: string, clear: { password: boolean; keyPassphrase: boolean }): SaveConnReq {
  return {
    id,
    name: f.name.trim(),
    group: f.group.trim(),
    color: f.color,
    note: f.note.trim(),
    production: f.production,
    kind: "ssh",
    host: f.host.trim(),
    port: Number(f.port),
    user: f.user.trim(),
    auth: f.auth,
    keyPath: f.auth === "key" ? f.keyPath.trim() : "",
    password: f.auth === "password" ? f.password : "",
    keyPassphrase: f.auth === "key" ? f.keyPassphrase : "",
    jumpId: f.jumpId,
    keepaliveSeconds: Number(f.keepaliveSeconds),
    initDir: f.initDir.trim(),
    incognito: f.incognito,
    clearPassword: clear.password,
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
  onChange,
  onClear,
}: {
  label: string;
  saved: boolean;
  cleared: boolean;
  value: string;
  hint?: string;
  onChange: (v: string) => void;
  onClear: (v: boolean) => void;
}) {
  const { t } = useI18n();
  const link = "ml-1.5 text-accent hover:underline";
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
        <button type="button" className={link} onClick={() => onClear(true)}>
          {t("editor.secretClear")}
        </button>
      </>
    )
  ) : null;
  return (
    <Field label={label} hint={savedHint ?? hint}>
      <PasswordInput
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

  useEffect(() => {
    if (!open) return;
    const next = toForm(conn);
    if (!conn && group) next.group = group;
    setForm(next);
    setErrors({});
    setClear({ password: false, keyPassphrase: false });
  }, [open, conn, group]);

  const f = form;
  const set = <K extends keyof Form>(k: K, v: Form[K]) => {
    setForm((x) => ({ ...x, [k]: v }));
    if (errors[k]) setErrors((e) => ({ ...e, [k]: undefined }));
  };

  const groups = useMemo(() => [...new Set([...loadGroups(), ...conns.map((c) => c.group).filter(Boolean)])] as string[], [conns, open]);

  // 可选的跳板机：编辑时排除自己和跳板链上经过自己的连接，避免成环。
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
    return conns.filter((c) => !(conn && passesSelf(c.id)));
  }, [conns, conn]);

  const pickKey = async () => {
    try {
      const p = await connApi.pickKeyFile();
      if (p) set("keyPath", p);
    } catch (e) {
      toast.error(e);
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
          <KindIcon kind="ssh" />
          {t(conn ? "editor.titleEdit" : "editor.titleNew", { kind: KIND_LABEL.ssh })}
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
            onChange={(v) => set("password", v)}
            onClear={(v) => setClear((c) => ({ ...c, password: v }))}
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
      </Section>

      <Section title={t("editor.section.advanced")}>
        <Field label={t("editor.jump")} hint={jumpOptions.length ? t("editor.jumpHint") : undefined}>
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
        <Row>
          <Field label={t("editor.initDir")}>
            <Input mono placeholder={t("editor.initDirPlaceholder")} value={f.initDir} onChange={(e) => set("initDir", e.target.value)} />
          </Field>
          <Field label={t("editor.keepalive")} error={errors.keepaliveSeconds} hint={t("editor.keepaliveHint")} className="!flex-none w-36">
            <Input value={f.keepaliveSeconds} invalid={!!errors.keepaliveSeconds} onChange={(e) => set("keepaliveSeconds", e.target.value)} />
          </Field>
        </Row>
      </Section>

      <Section title={t("editor.section.other")}>
        <SwitchRow
          title={t("editor.production")}
          desc={t("editor.productionDesc")}
          checked={f.production}
          onChange={(v) => set("production", v)}
        />
        <SwitchRow
          title={t("editor.incognito")}
          desc={t("editor.incognitoDesc")}
          checked={f.incognito}
          onChange={(v) => set("incognito", v)}
        />
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
