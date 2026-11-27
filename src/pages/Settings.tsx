import { ArrowLeft, ArrowUpCircle, FolderOpen, Monitor, Moon, RefreshCw, ScrollText, Sun } from "lucide-react";
import { useEffect, useState, type ReactNode } from "react";
import LogViewer from "../components/LogViewer";
import PageHeader from "../components/PageHeader";
import UpdateRouteMenu from "../components/UpdateRouteMenu";
import { Button, Card, Input, Segmented, Select, Switch } from "../components/ui";
import { settingsApi } from "../lib/api";
import { useI18n, type Language } from "../lib/i18n";
import { useSettings, useToast, useUpdate } from "../lib/store";
import type { ColorScheme, CursorStyle, LogLevel, SettingsInput, TerminalSettings, ThemeMode } from "../lib/types";

const FONT_SIZES = Array.from({ length: 25 }, (_, i) => i + 8);
const SCROLLBACKS = [1000, 5000, 10000, 20000, 50000, 100000];
const SCHEMES: { value: Exclude<ColorScheme, "auto">; label: string }[] = [
  { value: "oneDark", label: "One Dark" },
  { value: "dracula", label: "Dracula" },
  { value: "nord", label: "Nord" },
  { value: "solarizedDark", label: "Solarized Dark" },
  { value: "solarizedLight", label: "Solarized Light" },
];

/** 输入完成（失焦或回车）后才保存，避免每输入一个字符终端就重新排版。 */
function FontInput({ value, onCommit }: { value: string; onCommit: (v: string) => void }) {
  const { t } = useI18n();
  const [v, setV] = useState(value);
  useEffect(() => setV(value), [value]);
  const commit = (raw: string) => raw.trim() !== value && onCommit(raw.trim());
  return (
    <div className="w-56 shrink-0">
      <Input
        mono
        placeholder={t("settings.fontFamilyPlaceholder")}
        value={v}
        onChange={(e) => setV(e.target.value)}
        onBlur={(e) => commit(e.currentTarget.value)}
        onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
      />
    </div>
  );
}

function Group({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section>
      <h2 className="mb-2 px-1 text-xs font-semibold tracking-wide text-muted">{title}</h2>
      <Card className="divide-y divide-line [&>*]:px-4 [&>*]:py-3.5">{children}</Card>
    </section>
  );
}

function Row({ title, desc, children }: { title: string; desc?: ReactNode; children: ReactNode }) {
  return (
    <div className="flex items-center gap-4">
      <div className="min-w-0 flex-1">
        <div className="text-sm">{title}</div>
        {desc && <div className="mt-0.5 truncate text-xs text-muted">{desc}</div>}
      </div>
      {children}
    </div>
  );
}

export default function Settings({ onBack }: { onBack: () => void }) {
  const { t } = useI18n();
  const { settings: s, save } = useSettings();
  const toast = useToast();
  const { info: updateInfo, checking, check, setDialogOpen } = useUpdate();
  const newVersion = updateInfo?.hasUpdate ? updateInfo.latest : undefined;
  const [logOpen, setLogOpen] = useState(false);
  const header = (
    <PageHeader
      title={t("nav.settings")}
      actions={
        <Button variant="ghost" icon={<ArrowLeft size={15} />} onClick={onBack}>
          {t("nav.terminal")}
        </Button>
      }
    />
  );
  if (!s) return header;

  const update = (patch: Partial<SettingsInput>) => save(patch).catch(toast.error);
  const term = s.terminal;
  const updateTerm = (patch: Partial<TerminalSettings>) => update({ terminal: { ...term, ...patch } });
  // 配置文件里手动改成的值不在预设里时也显示出来。
  const scrollbacks = SCROLLBACKS.includes(term.scrollback) ? SCROLLBACKS : [...SCROLLBACKS, term.scrollback].sort((a, b) => a - b);

  return (
    <>
      {header}
      <div className="min-h-0 flex-1 overflow-y-auto px-6 pb-8">
        <div className="mx-auto flex max-w-2xl flex-col gap-6">
          <Group title={t("settings.appearance")}>
            <Row title={t("settings.theme")}>
              <Segmented<ThemeMode>
                value={s.theme}
                onChange={(theme) => update({ theme })}
                options={[
                  { value: "system", label: <><Monitor size={14} />{t("settings.theme.system")}</> },
                  { value: "light", label: <><Sun size={14} />{t("settings.theme.light")}</> },
                  { value: "dark", label: <><Moon size={14} />{t("settings.theme.dark")}</> },
                ]}
              />
            </Row>
            <Row title={t("settings.language")}>
              <Select<Language>
                className="w-36"
                value={s.language}
                onChange={(language) => update({ language })}
                options={[
                  { value: "system", label: t("settings.language.system") },
                  { value: "zh-CN", label: "简体中文" },
                  { value: "zh-TW", label: "繁體中文" },
                  { value: "en", label: "English" },
                ]}
              />
            </Row>
          </Group>

          <Group title={t("settings.terminal")}>
            <Row title={t("settings.fontFamily")} desc={t("settings.fontFamilyDesc")}>
              <FontInput value={term.fontFamily} onCommit={(fontFamily) => updateTerm({ fontFamily })} />
            </Row>
            <Row title={t("settings.fontSize")}>
              <Select
                className="w-24"
                value={String(term.fontSize)}
                onChange={(v) => updateTerm({ fontSize: Number(v) })}
                options={FONT_SIZES.map((n) => ({ value: String(n), label: String(n) }))}
              />
            </Row>
            <Row title={t("settings.colorScheme")}>
              <Select<ColorScheme>
                className="w-44"
                value={term.colorScheme}
                onChange={(colorScheme) => updateTerm({ colorScheme })}
                options={[{ value: "auto", label: t("settings.colorScheme.auto") }, ...SCHEMES]}
              />
            </Row>
            <Row title={t("settings.cursorStyle")}>
              <Segmented<CursorStyle>
                value={term.cursorStyle}
                onChange={(cursorStyle) => updateTerm({ cursorStyle })}
                options={[
                  { value: "block", label: t("settings.cursorStyle.block") },
                  { value: "bar", label: t("settings.cursorStyle.bar") },
                  { value: "underline", label: t("settings.cursorStyle.underline") },
                ]}
              />
            </Row>
            <Row title={t("settings.scrollback")} desc={t("settings.scrollbackDesc")}>
              <Select
                className="w-32"
                value={String(term.scrollback)}
                onChange={(v) => updateTerm({ scrollback: Number(v) })}
                options={scrollbacks.map((n) => ({ value: String(n), label: n.toLocaleString() }))}
              />
            </Row>
            <Row title={t("settings.copyOnSelect")} desc={t("settings.copyOnSelectDesc")}>
              <Switch label={t("settings.copyOnSelect")} checked={term.copyOnSelect} onChange={(copyOnSelect) => updateTerm({ copyOnSelect })} />
            </Row>
          </Group>

          <Group title={t("settings.logs")}>
            <Row title={t("settings.logLevel")} desc={t("settings.logLevelDesc")}>
              <Select<LogLevel>
                className="w-32"
                value={s.logLevel}
                onChange={(logLevel) => update({ logLevel })}
                options={[
                  { value: "debug", label: "Debug" },
                  { value: "info", label: "Info" },
                  { value: "warn", label: "Warn" },
                  { value: "error", label: "Error" },
                ]}
              />
            </Row>
            <Row title={t("settings.logDir")} desc={<span className="font-mono">{s.logDir}</span>}>
              <div className="flex shrink-0 gap-2">
                <Button size="sm" icon={<ScrollText size={13} />} onClick={() => setLogOpen(true)}>
                  {t("settings.viewLog")}
                </Button>
                <Button size="sm" icon={<FolderOpen size={13} />} onClick={() => settingsApi.openLogDir().catch(toast.error)}>
                  {t("common.open")}
                </Button>
              </div>
            </Row>
          </Group>

          <Group title={t("settings.data")}>
            <Row title={t("settings.dataDir")} desc={<span className="font-mono">{s.dataDir}</span>}>
              <Button size="sm" icon={<FolderOpen size={13} />} onClick={() => settingsApi.openDataDir().catch(toast.error)}>
                {t("common.open")}
              </Button>
            </Row>
            <div className="text-xs leading-relaxed text-muted">{t("settings.dataNote")}</div>
          </Group>

          <Group title={t("settings.about")}>
            <Row title="Rain Term" desc={newVersion ? t("update.found", { version: newVersion }) : t("settings.aboutDesc")}>
              <div className="flex items-center gap-3">
                <span className="relative font-mono text-xs text-muted">
                  v{s.version}
                  {newVersion && <span className="absolute -top-1 -right-2 size-2 rounded-full bg-red-500" />}
                </span>
                <UpdateRouteMenu onPick={check}>
                  {(open) => (
                    <Button
                      size="sm"
                      icon={<RefreshCw size={13} className={checking ? "animate-spin" : undefined} />}
                      disabled={checking}
                      onClick={open}
                      title={updateInfo?.checkedAt && t("update.lastChecked", { time: new Date(updateInfo.checkedAt).toLocaleString() })}
                    >
                      {t(checking ? "update.checking" : "update.check")}
                    </Button>
                  )}
                </UpdateRouteMenu>
                {newVersion && (
                  <Button size="sm" variant="primary" icon={<ArrowUpCircle size={13} />} onClick={() => setDialogOpen(true)}>
                    {t("update.update")}
                  </Button>
                )}
              </div>
            </Row>
          </Group>
        </div>
      </div>
      <LogViewer open={logOpen} onClose={() => setLogOpen(false)} />
    </>
  );
}
