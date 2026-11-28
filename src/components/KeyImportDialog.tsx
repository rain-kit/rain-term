import { readText } from "@tauri-apps/plugin-clipboard-manager";
import { ClipboardPaste, FileUp, FolderOpen, Loader2, X } from "lucide-react";
import { useEffect, useState } from "react";
import { keyApi } from "../lib/api";
import { useI18n } from "../lib/i18n";
import { useToast } from "../lib/store";
import type { SshKey } from "../lib/types";
import { Button, cx, Dialog, Field, Input, PasswordInput } from "./ui";

const textareaCls =
  "w-full resize-none rounded-lg border bg-surface px-3 py-2 font-mono text-xs leading-relaxed text-ink outline-none transition-colors placeholder:text-muted/60 focus:border-accent focus:ring-3 focus:ring-accent/15";

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

  const pasteClipboard = async () => {
    try {
      const text = await readText();
      if (!text) return;
      setContent(text);
      setError("");
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
      divided
      title={
        <span className="flex items-center gap-2">
          <span className="flex size-6 items-center justify-center rounded-md bg-accent/12 text-accent">
            <FileUp size={14} />
          </span>
          {t("keys.importTitle")}
        </span>
      }
      width="max-w-lg"
      footer={
        <>
          <Button onClick={onClose} disabled={busy}>
            {t("common.cancel")}
          </Button>
          <Button variant="primary" icon={busy ? <Loader2 size={14} className="animate-spin" /> : undefined} onClick={submit} disabled={busy}>
            {t("keys.import")}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-3.5">
        <Field label={t("keys.name")}>
          <Input
            autoFocus
            placeholder={t("keys.namePlaceholder")}
            value={name}
            onChange={(e) => setName(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && content.trim() && submit()}
          />
        </Field>
        <Field
          label={t("keys.privateKey")}
          error={error || undefined}
          hint={!error ? t("keys.privateKeyHint") : undefined}
          extra={
            <div className="-my-1 flex items-center gap-0.5">
              {content && (
                <Button
                  type="button"
                  size="sm"
                  variant="ghost"
                  icon={<X size={13} />}
                  onClick={() => {
                    setContent("");
                    setError("");
                  }}
                >
                  {t("keys.clear")}
                </Button>
              )}
              <Button type="button" size="sm" variant="ghost" icon={<ClipboardPaste size={13} />} onClick={pasteClipboard}>
                {t("terminal.paste")}
              </Button>
              <Button type="button" size="sm" variant="secondary" icon={<FolderOpen size={13} />} onClick={readFile}>
                {t("keys.readFile")}
              </Button>
            </div>
          }
        >
          <textarea
            rows={7}
            spellCheck={false}
            value={content}
            placeholder={"-----BEGIN OPENSSH PRIVATE KEY-----\n…\n-----END OPENSSH PRIVATE KEY-----"}
            onChange={(e) => {
              setContent(e.target.value);
              if (error) setError("");
            }}
            className={cx(textareaCls, error ? "border-red-500 focus:border-red-500 focus:ring-red-500/15" : "border-line")}
          />
        </Field>
        <Field label={t("keys.passphrase")} hint={t("keys.passphraseHint")}>
          <PasswordInput
            value={passphrase}
            onChange={(e) => setPassphrase(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && submit()}
          />
        </Field>
      </div>
    </Dialog>
  );
}
