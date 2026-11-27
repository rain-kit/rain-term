import { ShieldAlert, ShieldQuestion } from "lucide-react";
import { useI18n } from "../lib/i18n";
import { useHostKeys } from "../lib/store";
import { Button, Dialog } from "./ui";

/** 首次连接服务器时确认主机指纹或远程桌面证书；多个请求依次处理。 */
export default function HostKeyDialog() {
  const { t, tx } = useI18n();
  const { prompts, answer } = useHostKeys();
  const p = prompts[0];
  if (!p) return null;
  const rdp = p.kind === "rdp";
  const changed = rdp && !!p.previous;
  const args = {
    name: <span className="font-medium text-ink">{p.connName || p.host}</span>,
    host: <span className="font-mono text-ink">{p.host}</span>,
  };
  return (
    <Dialog
      open
      title={
        <span className="flex items-center gap-2">
          {changed ? <ShieldAlert size={18} className="text-red-500" /> : <ShieldQuestion size={18} className="text-amber-500" />}
          {t(changed ? "rdpCert.changedTitle" : rdp ? "rdpCert.title" : "hostKey.title")}
        </span>
      }
      footer={
        <>
          <Button onClick={() => answer(p, false)}>{t("hostKey.reject")}</Button>
          <Button variant={changed ? "danger" : "primary"} onClick={() => answer(p, true)}>
            {t(changed ? "rdpCert.update" : "hostKey.trust")}
          </Button>
        </>
      }
    >
      <p className="text-[13px] leading-relaxed text-muted">
        {tx(changed ? "rdpCert.changedMessage" : rdp ? "rdpCert.message" : "hostKey.message", args)}
      </p>
      <div className="mt-3 rounded-lg bg-subtle p-3">
        {rdp && p.keyType && <div className="mb-2 text-xs break-all text-muted">{t("rdpCert.subject", { subject: p.keyType })}</div>}
        <div className="text-xs text-muted">{rdp ? t("rdpCert.fingerprint") : t("hostKey.fingerprint", { type: p.keyType })}</div>
        <div className="selectable mt-1 font-mono text-[13px] break-all">{p.fingerprint}</div>
        {changed && (
          <>
            <div className="mt-2 text-xs text-muted">{t("rdpCert.previous")}</div>
            <div className="selectable mt-1 font-mono text-[13px] break-all text-muted line-through">{p.previous}</div>
          </>
        )}
      </div>
      <p className="mt-3 text-xs text-muted">{t(rdp ? "rdpCert.note" : "hostKey.note")}</p>
      {prompts.length > 1 && <p className="mt-2 text-xs text-muted">{t("hostKey.pending", { count: prompts.length - 1 })}</p>}
    </Dialog>
  );
}
