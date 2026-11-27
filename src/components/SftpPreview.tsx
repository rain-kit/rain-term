import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { Copy, Download, LoaderCircle, RotateCw } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { errorMessage } from "../lib/api";
import { formatBytes } from "../lib/format";
import { useI18n } from "../lib/i18n";
import { useToast } from "../lib/store";
import type { FileEntry, FilePreview } from "../lib/types";
import { Button, cx, Dialog, Switch } from "./ui";

const WRAP_KEY = "sftp.previewWrap";
/** 与后端 service/sftp.rs 的 PREVIEW_LIMIT 一致。 */
const PREVIEW_LIMIT = 1024 * 1024;

const TEXT_EXTS = new Set(
  (
    "txt text log out md markdown rst adoc yaml yml json json5 jsonc toml ini cfg conf config cnf properties prop env " +
    "xml xsd xsl html htm css scss sass less csv tsv sql sh bash zsh fish ksh csh ps1 psm1 bat cmd " +
    "py rb pl pm php lua go rs java kt kts scala groovy gradle c h cc cpp cxx hpp hh cs swift dart " +
    "js mjs cjs jsx ts mts cts tsx vue svelte r tf tfvars hcl proto graphql gql " +
    "service socket timer mount target path slice repo list sources diff patch " +
    "pem crt cer csr pub asc gitignore dockerignore editorconfig gitattributes lock vim srt"
  ).split(" "),
);
const TEXT_NAMES = new Set(
  (
    "dockerfile containerfile makefile gnumakefile cmakelists.txt readme license copying changelog authors " +
    "vagrantfile jenkinsfile procfile gemfile rakefile caddyfile hosts hostname fstab crontab passwd group " +
    "sudoers authorized_keys known_hosts config environment profile"
  ).split(" "),
);

/** 按文件名判断是不是常见的文本文件，双击时直接预览；其他文件仍可以从右键菜单预览。 */
export function previewable(name: string) {
  const n = name.toLowerCase();
  if (TEXT_NAMES.has(n) || /\.(log|txt|out)\.\d+$/.test(n)) return true;
  const dot = n.lastIndexOf(".");
  // .bashrc、.env 这类只有前导点的文件
  if (dot === 0) return true;
  return dot > 0 && TEXT_EXTS.has(n.slice(dot + 1));
}

/** 预览 SFTP 上的文本文件：只读，大文件只显示开头部分。 */
export default function SftpPreview({
  entry,
  read,
  onDownload,
  onClose,
}: {
  entry: FileEntry | null;
  read: (path: string) => Promise<FilePreview>;
  onDownload: (e: FileEntry) => void;
  onClose: () => void;
}) {
  const { t } = useI18n();
  const toast = useToast();
  const [data, setData] = useState<FilePreview | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [wrap, setWrap] = useState(() => localStorage.getItem(WRAP_KEY) === "1");
  const seq = useRef(0);
  const textEl = useRef<HTMLPreElement>(null);

  const load = (path: string) => {
    const my = ++seq.current;
    setLoading(true);
    setError(null);
    read(path)
      .then((d) => my === seq.current && setData(d))
      .catch((e) => {
        if (my !== seq.current) return;
        setData(null);
        setError(errorMessage(e));
      })
      .finally(() => my === seq.current && setLoading(false));
  };

  useEffect(() => {
    setData(null);
    if (entry) load(entry.path);
    else seq.current++;
  }, [entry?.path]);

  const lines = useMemo(() => {
    if (!data?.text) return 0;
    const n = data.text.split("\n").length;
    return data.text.endsWith("\n") ? n - 1 : n;
  }, [data]);
  const gutter = useMemo(() => Array.from({ length: lines }, (_, i) => i + 1).join("\n"), [lines]);

  const toggleWrap = (v: boolean) => {
    setWrap(v);
    localStorage.setItem(WRAP_KEY, v ? "1" : "0");
  };

  const copy = () => data && writeText(data.text).then(() => toast.success(t("sftp.contentCopied")), toast.error);

  return (
    <Dialog
      open={!!entry}
      onClose={onClose}
      width="max-w-5xl"
      title={
        <span className="break-all" title={entry?.path}>
          {entry?.name}
        </span>
      }
      footer={
        <>
          <div className="mr-auto flex min-w-0 items-center gap-3 text-xs text-muted">
            <label className="flex shrink-0 items-center gap-2">
              <Switch checked={wrap} onChange={toggleWrap} label={t("sftp.previewWrap")} />
              {t("sftp.previewWrap")}
            </label>
            {data && (
              <span className="truncate tabular-nums">
                {formatBytes(data.size)} · {t("sftp.previewLines", { count: lines })}
                {data.truncated && (
                  <span className="text-amber-600 dark:text-amber-400">
                    {" · "}
                    {t("sftp.previewTruncated", { size: formatBytes(PREVIEW_LIMIT) })}
                  </span>
                )}
              </span>
            )}
          </div>
          <Button icon={<RotateCw size={14} className={cx(loading && "animate-spin")} />} disabled={loading} onClick={() => entry && load(entry.path)}>
            {t("sftp.refresh")}
          </Button>
          <Button icon={<Copy size={14} />} disabled={!data} onClick={copy}>
            {t("sftp.copyContent")}
          </Button>
          <Button icon={<Download size={14} />} onClick={() => entry && onDownload(entry)}>
            {t("sftp.download")}
          </Button>
        </>
      }
    >
      <div
        tabIndex={0}
        onKeyDown={(e) => {
          if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "a" && textEl.current) {
            e.preventDefault();
            getSelection()?.selectAllChildren(textEl.current);
          }
        }}
        className="h-[65vh] overflow-auto rounded-lg border border-line bg-subtle/40 font-mono text-xs leading-5 outline-none"
      >
        {data ? (
          <div className={cx("flex min-h-full", wrap ? "w-full" : "w-max min-w-full")}>
            {!wrap && lines > 0 && (
              <pre className="sticky left-0 shrink-0 border-r border-line bg-subtle px-2 py-2 text-right text-muted select-none">{gutter}</pre>
            )}
            <pre ref={textEl} className={cx("min-w-0 flex-1 px-3 py-2", wrap ? "break-all whitespace-pre-wrap" : "whitespace-pre")}>
              {data.text || <span className="font-sans text-muted">{t("sftp.previewEmpty")}</span>}
            </pre>
          </div>
        ) : (
          <div className="flex h-full items-center justify-center gap-2 px-6 text-center font-sans text-sm">
            {error ? (
              <span className="text-red-600 dark:text-red-400">{error}</span>
            ) : (
              <>
                <LoaderCircle size={15} className="animate-spin text-muted" />
                <span className="text-muted">{t("sftp.previewLoading")}</span>
              </>
            )}
          </div>
        )}
      </div>
    </Dialog>
  );
}
