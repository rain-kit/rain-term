import { Download, Loader2, Upload } from "lucide-react";
import { useEffect, useState } from "react";
import { connApi, ErrorCode, errorCode } from "../lib/api";
import { useI18n } from "../lib/i18n";
import { useToast } from "../lib/store";
import { Button, Dialog, Field, PasswordInput } from "./ui";

/** 导出时 ids 为空表示全部；导入时 path 是已经选好并检查过格式的文件。 */
export type BackupTask = { mode: "export"; ids: string[] } | { mode: "import"; path: string };

/** 导出时设置、导入时输入导出密码：私钥和各类密码用它加密后放在导出文件里。 */
export default function BackupDialog({ task, onClose }: { task: BackupTask | null; onClose: () => void }) {
  const { t } = useI18n();
  const toast = useToast();
  const [password, setPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const [error, setError] = useState("");
  const [confirmError, setConfirmError] = useState("");
  const [busy, setBusy] = useState(false);
  const exporting = task?.mode === "export";

  useEffect(() => {
    setPassword("");
    setConfirm("");
    setError("");
    setConfirmError("");
  }, [task]);

  const submit = async () => {
    if (!task || busy) return;
    if (!password) {
      setError(t("backup.passwordRequired"));
      return;
    }
    if (task.mode === "export" && password !== confirm) {
      setConfirmError(t("backup.passwordMismatch"));
      return;
    }
    setBusy(true);
    try {
      if (task.mode === "export") {
        const r = await connApi.exportTo(task.ids, password);
        if (r) toast.success(t("backup.exported", { ...r }));
      } else {
        const { settings, ...counts } = await connApi.importFrom(task.path, password);
        toast.success(t(settings ? "backup.importedWithSettings" : "backup.imported", counts));
      }
      onClose();
    } catch (e) {
      if (errorCode(e) === ErrorCode.importWrongPassword) setError(t("err.importWrongPassword"));
      else toast.error(e);
    } finally {
      setBusy(false);
    }
  };

  const fileName = task?.mode === "import" ? (task.path.split(/[\\/]/).pop() ?? task.path) : "";
  const desc = !task
    ? ""
    : task.mode === "import"
      ? t("backup.importDesc", { file: fileName })
      : task.ids.length > 0
        ? t("backup.exportSelectedDesc")
        : t("backup.exportAllDesc");

  return (
    <Dialog
      open={!!task}
      onClose={busy ? undefined : onClose}
      title={
        <span className="flex items-center gap-2">
          <span className="flex size-6 items-center justify-center rounded-md bg-accent/12 text-accent">
            {exporting ? <Upload size={14} /> : <Download size={14} />}
          </span>
          {t(exporting ? "backup.exportTitle" : "backup.importTitle")}
        </span>
      }
      width="max-w-md"
      footer={
        <>
          <Button onClick={onClose} disabled={busy}>
            {t("common.cancel")}
          </Button>
          <Button variant="primary" icon={busy ? <Loader2 size={14} className="animate-spin" /> : undefined} onClick={submit} disabled={busy}>
            {t(exporting ? "backup.export" : "backup.import")}
          </Button>
        </>
      }
    >
      <p className="mb-3 text-sm text-muted">{desc}</p>
      <div className="flex flex-col gap-3">
        <Field label={t("backup.password")} error={error || undefined} hint={!error && exporting ? t("backup.passwordHint") : undefined}>
          <PasswordInput
            autoFocus
            value={password}
            invalid={!!error}
            onChange={(e) => {
              setPassword(e.target.value);
              setError("");
            }}
            onKeyDown={(e) => e.key === "Enter" && submit()}
          />
        </Field>
        {exporting && (
          <Field label={t("backup.passwordConfirm")} error={confirmError || undefined}>
            <PasswordInput
              value={confirm}
              invalid={!!confirmError}
              onChange={(e) => {
                setConfirm(e.target.value);
                setConfirmError("");
              }}
              onKeyDown={(e) => e.key === "Enter" && submit()}
            />
          </Field>
        )}
      </div>
    </Dialog>
  );
}
