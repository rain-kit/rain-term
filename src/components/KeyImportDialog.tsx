import { FolderOpen } from "lucide-react";
import { useEffect, useState } from "react";
import { keyApi } from "../lib/api";
import { useI18n } from "../lib/i18n";
import { useToast } from "../lib/store";
import type { SshKey } from "../lib/types";
import { Button, cx, Dialog, Field, Input, PasswordInput } from "./ui";

const textareaCls =
  "w-full resize-none rounded-lg border bg-surface px-2.5 py-1.5 font-mono text-xs text-ink outline-none transition-colors placeholder:text-muted/60 focus:border-accent focus:ring-3 focus:ring-accent/15";

/** 导入私钥：从文件读取或直接粘贴，私钥内容保存在程序里，之后不再需要原来的文件。 */
export default function KeyImportDialog({ open, onClose, onDone }: { open: boolean; onClose: () => void; onDone?: (k: SshKey) => void }) {
  const { t } = useI18n();
  const toast = useToast();
  const [name, setName] = useState("");
  const [content, setContent] = useState("");
  const [passphrase, setPassphrase] = useState("");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!open) return;
    setName("");
    setContent("");
    setPassphrase("");
    setError("");
  }, [open]);

  const readFile = async () => {
    try {
      const f = await keyApi.readFile();
      if (!f) return;
      setContent(f.content);
      setError("");
      setName((n) => n || f.name);
    } catch (e) {
      toast.error(e);
    }
  };

  const submit = async () => {
    if (!content.trim()) {
      setError(t("keys.contentRequired"));
      return;
    }
    setBusy(true);
    try {
      const k = await keyApi.import({ name: name.trim(), content, passphrase });
      toast.success(t("keys.imported", { name: k.name }));
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
      title={t("keys.importTitle")}
      width="max-w-lg"
      footer={
        <>
          <Button onClick={onClose} disabled={busy}>
            {t("common.cancel")}
          </Button>
          <Button variant="primary" onClick={submit} disabled={busy}>
            {t("keys.import")}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-3 pb-2">
        <Field label={t("keys.name")}>
          <Input placeholder={t("keys.namePlaceholder")} value={name} onChange={(e) => setName(e.target.value)} />
        </Field>
        <div className="flex flex-col gap-1">
          <div className="flex items-center">
            <span className="flex-1 text-xs font-medium text-muted">{t("keys.privateKey")}</span>
            <Button size="sm" variant="ghost" icon={<FolderOpen size={13} />} onClick={readFile}>
              {t("keys.readFile")}
            </Button>
          </div>
          <textarea
            rows={8}
            spellCheck={false}
            value={content}
            placeholder={"-----BEGIN OPENSSH PRIVATE KEY-----\n…\n-----END OPENSSH PRIVATE KEY-----"}
            onChange={(e) => {
              setContent(e.target.value);
              setError("");
            }}
            className={cx(textareaCls, error ? "border-red-500" : "border-line")}
          />
          {error ? (
            <span className="text-xs text-red-600 dark:text-red-400">{error}</span>
          ) : (
            <span className="text-xs text-muted">{t("keys.privateKeyHint")}</span>
          )}
        </div>
        <Field label={t("keys.passphrase")} hint={t("keys.passphraseHint")}>
          <PasswordInput value={passphrase} onChange={(e) => setPassphrase(e.target.value)} />
        </Field>
      </div>
    </Dialog>
  );
}
