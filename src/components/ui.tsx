import { Check, ChevronDown, Eye, EyeOff, VenetianMask, X } from "lucide-react";
import {
  forwardRef,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type ButtonHTMLAttributes,
  type InputHTMLAttributes,
  type ReactNode,
  type RefObject,
} from "react";
import { createPortal } from "react-dom";
import { useI18n } from "../lib/i18n";

export const cx = (...c: (string | false | null | undefined)[]) => c.filter(Boolean).join(" ");

// ---------- 按钮 ----------

type Variant = "primary" | "secondary" | "ghost" | "danger";

const variants: Record<Variant, string> = {
  primary: "bg-accent text-white hover:bg-accent-hover shadow-sm",
  secondary: "bg-surface border border-line hover:bg-subtle text-ink",
  ghost: "hover:bg-subtle text-ink",
  danger: "bg-red-600 text-white hover:bg-red-700 shadow-sm",
};

interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: Variant;
  size?: "sm" | "md";
  icon?: ReactNode;
}

export const Button = forwardRef<HTMLButtonElement, ButtonProps>(function Button(
  { variant = "secondary", size = "md", icon, className, children, ...rest },
  ref,
) {
  return (
    <button
      ref={ref}
      className={cx(
        "inline-flex items-center justify-center gap-1.5 rounded-lg font-medium whitespace-nowrap transition-colors",
        "disabled:opacity-50 disabled:pointer-events-none focus-visible:outline-2 focus-visible:outline-accent/60",
        size === "sm" ? "h-7 px-2.5 text-xs" : "h-8.5 px-3.5 text-sm",
        variants[variant],
        className,
      )}
      {...rest}
    >
      {icon}
      {children}
    </button>
  );
});

interface IconButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  label: string;
  size?: "xs" | "sm" | "md";
  tone?: "default" | "danger";
}

export const IconButton = forwardRef<HTMLButtonElement, IconButtonProps>(function IconButton(
  { label, size = "md", tone = "default", className, children, ...rest },
  ref,
) {
  return (
    <button
      ref={ref}
      title={label}
      aria-label={label}
      className={cx(
        "inline-flex items-center justify-center text-muted transition-colors disabled:opacity-40 disabled:pointer-events-none",
        size === "xs" ? "size-6 rounded-md" : size === "sm" ? "size-7 rounded-lg" : "size-8 rounded-lg",
        tone === "danger" ? "hover:bg-red-500/10 hover:text-red-600" : "hover:bg-subtle hover:text-ink",
        className,
      )}
      {...rest}
    >
      {children}
    </button>
  );
});

// ---------- 表单 ----------

export function Field({
  label,
  extra,
  hint,
  error,
  className,
  children,
}: {
  label?: ReactNode;
  extra?: ReactNode;
  hint?: ReactNode;
  error?: string;
  className?: string;
  children: ReactNode;
}) {
  return (
    <div className={cx("flex flex-col gap-1 min-w-0", className)}>
      {(label || extra) && (
        <div className="flex items-center justify-between gap-2">
          {label && <span className="text-xs font-medium text-muted">{label}</span>}
          {extra}
        </div>
      )}
      {children}
      {error ? (
        <span className="text-xs text-red-600 dark:text-red-400">{error}</span>
      ) : hint ? (
        <span className="text-xs text-muted">{hint}</span>
      ) : null}
    </div>
  );
}

interface InputProps extends InputHTMLAttributes<HTMLInputElement> {
  invalid?: boolean;
  mono?: boolean;
  suffix?: ReactNode;
}

const inputBase =
  "h-8.5 w-full rounded-lg border bg-surface px-2.5 text-sm text-ink outline-none transition-colors placeholder:text-muted/60 focus:border-accent focus:ring-3 focus:ring-accent/15 disabled:opacity-60";

export const Input = forwardRef<HTMLInputElement, InputProps>(function Input(
  { invalid, mono, suffix, className, ...rest },
  ref,
) {
  const input = (
    <input
      ref={ref}
      spellCheck={false}
      autoComplete="off"
      className={cx(
        inputBase,
        invalid ? "border-red-500 focus:border-red-500 focus:ring-red-500/15" : "border-line",
        mono && "font-mono text-[13px]",
        suffix ? "pr-9" : "",
        className,
      )}
      {...rest}
    />
  );
  if (!suffix) return input;
  return (
    <div className="relative">
      {input}
      <div className="absolute inset-y-0 right-1 flex items-center">{suffix}</div>
    </div>
  );
});

export function PasswordInput(props: Omit<InputProps, "type" | "suffix">) {
  const { t } = useI18n();
  const [show, setShow] = useState(false);
  return (
    <Input
      {...props}
      type={show ? "text" : "password"}
      suffix={
        <IconButton label={t(show ? "common.hide" : "common.show")} size="sm" type="button" onClick={() => setShow((v) => !v)}>
          {show ? <EyeOff size={15} /> : <Eye size={15} />}
        </IconButton>
      }
    />
  );
}

export interface SelectOption<T extends string = string> {
  value: T;
  label: string;
  /** 右侧辅助文字（如算法、地址、认证方式）。 */
  sub?: string;
  icon?: ReactNode;
}

export function Select<T extends string>({
  value,
  options,
  onChange,
  invalid,
  disabled,
  className,
}: {
  value: T;
  options: SelectOption<T>[];
  onChange: (v: T) => void;
  invalid?: boolean;
  disabled?: boolean;
  className?: string;
}) {
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(0);
  const anchor = useRef<HTMLButtonElement>(null);
  const menu = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<{ top: number; left: number; width: number }>({ top: 0, left: 0, width: 0 });

  const currentIdx = Math.max(0, options.findIndex((o) => o.value === value));
  const current = options[currentIdx] ?? options[0];

  useLayoutEffect(() => {
    if (!open || !anchor.current || !menu.current) return;
    const a = anchor.current.getBoundingClientRect();
    const m = menu.current.getBoundingClientRect();
    const width = Math.max(a.width, Math.min(m.width, window.innerWidth - 16));
    let top = a.bottom + 4;
    if (top + m.height > window.innerHeight - 8 && a.top - m.height - 4 >= 8) {
      top = a.top - m.height - 4;
    }
    const left = Math.max(8, Math.min(a.left, window.innerWidth - width - 8));
    setPos({ top, left, width });
  }, [open, options.length]);

  useEffect(() => {
    if (!open) return;
    setActive(currentIdx);
    requestAnimationFrame(() => {
      menu.current?.querySelector(`[data-idx="${currentIdx}"]`)?.scrollIntoView({ block: "nearest" });
    });
  }, [open, currentIdx]);

  useDismiss(open, menu, () => setOpen(false), anchor);

  useEffect(() => {
    if (!open || !options.length) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        e.stopPropagation();
        const next = (active + (e.key === "ArrowDown" ? 1 : -1) + options.length) % options.length;
        setActive(next);
        menu.current?.querySelector(`[data-idx="${next}"]`)?.scrollIntoView({ block: "nearest" });
      } else if (e.key === "Enter") {
        e.preventDefault();
        e.stopPropagation();
        const picked = options[active];
        if (picked) onChange(picked.value);
        setOpen(false);
        anchor.current?.focus();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [open, active, options, onChange]);

  return (
    <div className={cx("relative", className)}>
      <button
        ref={anchor}
        type="button"
        disabled={disabled}
        onClick={() => setOpen((v) => !v)}
        onKeyDown={(e) => {
          if (!open && (e.key === "ArrowDown" || e.key === "ArrowUp" || e.key === " ")) {
            e.preventDefault();
            setOpen(true);
          }
        }}
        className={cx(
          inputBase,
          "flex items-center gap-2 pr-8 text-left",
          invalid ? "border-red-500 focus:border-red-500 focus:ring-red-500/15" : open ? "border-accent ring-3 ring-accent/15" : "border-line",
        )}
      >
        {current?.icon && <span className="shrink-0 text-muted">{current.icon}</span>}
        <span className={cx("min-w-0 flex-1 truncate", !current?.value && options[0]?.value === "" && "text-muted")}>
          {current?.label ?? ""}
        </span>
        {current?.sub && <span className="shrink-0 truncate max-w-36 font-mono text-xs text-muted">{current.sub}</span>}
      </button>
      <ChevronDown
        size={15}
        className={cx("pointer-events-none absolute right-2.5 top-1/2 -translate-y-1/2 text-muted transition-transform", open && "rotate-180")}
      />
      {open &&
        createPortal(
          <div
            ref={menu}
            style={{ top: pos.top, left: pos.left, minWidth: pos.width, maxWidth: "min(28rem, calc(100vw - 16px))" }}
            className="fixed z-50 max-h-60 overflow-y-auto animate-pop rounded-xl border border-line bg-surface p-1 shadow-xl"
          >
            {options.map((o, i) => {
              const selected = o.value === value;
              return (
                <button
                  key={`${i}:${o.value}`}
                  type="button"
                  data-idx={i}
                  onMouseMove={() => i !== active && setActive(i)}
                  onClick={() => {
                    onChange(o.value);
                    setOpen(false);
                  }}
                  className={cx(
                    "flex w-full items-center gap-2 rounded-lg px-2.5 py-1.5 text-left text-[13px] transition-colors",
                    i === active ? "bg-subtle text-ink" : "text-ink",
                    selected && "font-medium",
                  )}
                >
                  {o.icon && <span className="shrink-0 text-muted">{o.icon}</span>}
                  <span className={cx("min-w-0 flex-1 truncate", !o.value && i === 0 && "text-muted")}>{o.label}</span>
                  {o.sub && <span className="shrink-0 truncate max-w-44 font-mono text-[11px] text-muted">{o.sub}</span>}
                  <span className="flex w-4 shrink-0 justify-center text-accent">{selected && <Check size={14} />}</span>
                </button>
              );
            })}
          </div>,
          document.body,
        )}
    </div>
  );
}

/** 既可自由输入，又可从下拉列表选择已有项（如连接分组）。 */
export function ComboInput({
  value,
  options,
  placeholder,
  emptyLabel,
  onChange,
  className,
}: {
  value: string;
  options: string[];
  placeholder?: string;
  /** 下拉顶部用于清空的选项文案（如“不分组”）。 */
  emptyLabel?: string;
  onChange: (v: string) => void;
  className?: string;
}) {
  const [open, setOpen] = useState(false);
  const wrap = useRef<HTMLDivElement>(null);
  const menu = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<{ top: number; left: number; width: number }>({ top: 0, left: 0, width: 0 });

  useLayoutEffect(() => {
    if (!open || !wrap.current || !menu.current) return;
    const a = wrap.current.getBoundingClientRect();
    const m = menu.current.getBoundingClientRect();
    let top = a.bottom + 4;
    if (top + m.height > window.innerHeight - 8 && a.top - m.height - 4 >= 8) {
      top = a.top - m.height - 4;
    }
    setPos({ top, left: Math.max(8, Math.min(a.left, window.innerWidth - a.width - 8)), width: a.width });
  }, [open, options.length]);

  useDismiss(open, menu, () => setOpen(false), wrap);

  const hasMenu = options.length > 0;
  return (
    <div ref={wrap} className={cx("relative", className)}>
      <Input
        placeholder={placeholder}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        suffix={
          hasMenu ? (
            <IconButton label={placeholder ?? ""} size="sm" type="button" tabIndex={-1} onClick={() => setOpen((v) => !v)}>
              <ChevronDown size={15} className={cx("transition-transform", open && "rotate-180")} />
            </IconButton>
          ) : undefined
        }
      />
      {open &&
        hasMenu &&
        createPortal(
          <div
            ref={menu}
            style={{ top: pos.top, left: pos.left, width: pos.width }}
            className="fixed z-50 max-h-52 overflow-y-auto animate-pop rounded-xl border border-line bg-surface p-1 shadow-xl"
          >
            {emptyLabel && (
              <button
                type="button"
                onClick={() => {
                  onChange("");
                  setOpen(false);
                }}
                className={cx(
                  "flex w-full items-center gap-2 rounded-lg px-2.5 py-1.5 text-left text-[13px] text-muted hover:bg-subtle hover:text-ink",
                  !value.trim() && "font-medium text-ink",
                )}
              >
                <span className="min-w-0 flex-1 truncate">{emptyLabel}</span>
                <span className="flex w-4 shrink-0 justify-center text-accent">{!value.trim() && <Check size={14} />}</span>
              </button>
            )}
            {options.map((g) => {
              const selected = value.trim() === g;
              return (
                <button
                  key={g}
                  type="button"
                  onClick={() => {
                    onChange(g);
                    setOpen(false);
                  }}
                  className={cx(
                    "flex w-full items-center gap-2 rounded-lg px-2.5 py-1.5 text-left text-[13px] hover:bg-subtle",
                    selected && "font-medium",
                  )}
                >
                  <span className="min-w-0 flex-1 truncate">{g}</span>
                  <span className="flex w-4 shrink-0 justify-center text-accent">{selected && <Check size={14} />}</span>
                </button>
              );
            })}
          </div>,
          document.body,
        )}
    </div>
  );
}

export function Switch({
  checked,
  onChange,
  disabled,
  label,
}: {
  checked: boolean;
  onChange: (v: boolean) => void;
  disabled?: boolean;
  label?: string;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className={cx(
        "relative inline-flex h-5 w-9 shrink-0 items-center rounded-full transition-colors disabled:opacity-50",
        checked ? "bg-accent" : "bg-zinc-300 dark:bg-zinc-600",
      )}
    >
      <span
        className={cx(
          "inline-block size-4 rounded-full bg-white shadow transition-transform",
          checked ? "translate-x-4.5" : "translate-x-0.5",
        )}
      />
    </button>
  );
}

/** 带标题和说明的开关行。 */
export function SwitchRow({
  title,
  badge,
  desc,
  checked,
  onChange,
  disabled,
}: {
  title: string;
  badge?: ReactNode;
  desc?: ReactNode;
  checked: boolean;
  onChange: (v: boolean) => void;
  disabled?: boolean;
}) {
  return (
    <div className="flex items-center gap-4">
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-1.5 text-sm">
          <span>{title}</span>
          {badge}
        </div>
        {desc && <div className="mt-0.5 text-xs text-muted">{desc}</div>}
      </div>
      <Switch checked={checked} onChange={onChange} disabled={disabled} label={title} />
    </div>
  );
}

export function Segmented<T extends string>({
  value,
  options,
  onChange,
  className,
}: {
  value: T;
  options: { value: T; label: ReactNode }[];
  onChange: (v: T) => void;
  className?: string;
}) {
  return (
    <div className={cx("inline-flex h-8.5 rounded-lg bg-subtle p-0.5", className)}>
      {options.map((o) => (
        <button
          key={o.value}
          type="button"
          onClick={() => onChange(o.value)}
          className={cx(
            "flex items-center gap-1.5 rounded-md px-3 text-[13px] whitespace-nowrap transition-colors",
            value === o.value ? "bg-surface text-ink shadow-sm font-medium" : "text-muted hover:text-ink",
          )}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

// ---------- 容器 ----------

export function Card({ className, children }: { className?: string; children: ReactNode }) {
  return <div className={cx("rounded-xl border border-line bg-surface", className)}>{children}</div>;
}

/** 名称旁边的灰色小标签。 */
export function Tag({ children }: { children: ReactNode }) {
  return <span className="flex shrink-0 items-center gap-0.5 rounded bg-subtle px-1.5 text-[10px] leading-4 font-medium text-muted">{children}</span>;
}

/** 连接名旁边的“生产”标记。 */
export function ProdBadge() {
  const { t } = useI18n();
  return (
    <span className="shrink-0 rounded bg-red-500/12 px-1 text-[10px] leading-4 font-semibold text-red-600 dark:bg-red-500/20 dark:text-red-400">
      {t("conn.production")}
    </span>
  );
}

/** 连接名旁边的“无痕”标记。 */
export function IncognitoBadge() {
  const { t } = useI18n();
  return (
    <span
      title={t("conn.incognitoHint")}
      className="flex shrink-0 items-center gap-0.5 rounded bg-indigo-500/12 px-1 text-[10px] leading-4 font-semibold text-indigo-600 dark:bg-indigo-500/20 dark:text-indigo-400"
    >
      <VenetianMask size={11} />
      {t("conn.incognito")}
    </span>
  );
}

export function Empty({
  icon,
  title,
  desc,
  action,
  compact,
}: {
  icon: ReactNode;
  title: string;
  desc?: string;
  action?: ReactNode;
  compact?: boolean;
}) {
  return (
    <div className={cx("flex flex-col items-center justify-center text-center", compact ? "py-10" : "py-16")}>
      <div className="mb-3 flex size-12 items-center justify-center rounded-2xl bg-subtle text-muted">{icon}</div>
      <div className="font-medium">{title}</div>
      {desc && <div className="mt-1 max-w-sm text-xs text-muted">{desc}</div>}
      {action && <div className="mt-4">{action}</div>}
    </div>
  );
}

// ---------- 对话框 ----------

/** 打开着的对话框，按打开顺序排列；对话框里还能再打开对话框，Esc 只关闭最上面的一个。 */
const openDialogs: symbol[] = [];

export function Dialog({
  open,
  onClose,
  title,
  width = "max-w-md",
  divided,
  bodyClassName,
  children,
  footer,
}: {
  open: boolean;
  onClose?: () => void;
  title: ReactNode;
  width?: string;
  /** 顶栏和底栏带分隔线，适合较复杂的表单和管理窗口。 */
  divided?: boolean;
  bodyClassName?: string;
  children: ReactNode;
  footer?: ReactNode;
}) {
  const { t } = useI18n();
  const close = useRef(onClose);
  close.current = onClose;
  useEffect(() => {
    if (!open) return;
    const id = Symbol();
    openDialogs.push(id);
    const fn = (e: KeyboardEvent) => e.key === "Escape" && openDialogs.at(-1) === id && close.current?.();
    window.addEventListener("keydown", fn);
    return () => {
      openDialogs.splice(openDialogs.indexOf(id), 1);
      window.removeEventListener("keydown", fn);
    };
  }, [open]);
  if (!open) return null;
  return createPortal(
    <div className="fixed inset-0 z-50 flex items-center justify-center p-6">
      <div className="absolute inset-0 animate-fade bg-black/40 dark:bg-black/60" onMouseDown={onClose} />
      <div
        role="dialog"
        aria-modal="true"
        className={cx(
          "relative flex max-h-full w-full animate-pop flex-col overflow-hidden rounded-2xl border border-line bg-surface shadow-2xl",
          width,
        )}
      >
        <div className={cx("flex shrink-0 items-center gap-2 px-5", divided ? "border-b border-line py-3.5" : "pt-4 pb-2")}>
          <div className="min-w-0 flex-1 text-base font-semibold">{title}</div>
          {onClose && (
            <IconButton label={t("common.close")} size="sm" onClick={onClose}>
              <X size={16} />
            </IconButton>
          )}
        </div>
        <div className={cx("min-h-0 flex-1 overflow-y-auto", bodyClassName ?? (divided ? "px-5 py-4" : "px-5 py-2"))}>{children}</div>
        {footer && (
          <div className={cx("flex shrink-0 items-center justify-end gap-2 px-5", divided ? "border-t border-line bg-subtle/30 py-3" : "pt-2 pb-4")}>
            {footer}
          </div>
        )}
      </div>
    </div>,
    document.body,
  );
}

export function Confirm({
  open,
  title,
  message,
  confirmText,
  danger,
  onConfirm,
  onClose,
}: {
  open: boolean;
  title: string;
  message: ReactNode;
  confirmText?: string;
  danger?: boolean;
  onConfirm: () => void;
  onClose: () => void;
}) {
  const { t } = useI18n();
  return (
    <Dialog
      open={open}
      onClose={onClose}
      title={title}
      width="max-w-sm"
      footer={
        <>
          <Button onClick={onClose}>{t("common.cancel")}</Button>
          <Button
            variant={danger ? "danger" : "primary"}
            onClick={() => {
              onConfirm();
              onClose();
            }}
          >
            {confirmText ?? t("common.confirm")}
          </Button>
        </>
      }
    >
      <div className="text-sm text-muted">{message}</div>
    </Dialog>
  );
}

// ---------- 下拉菜单 ----------

export interface MenuItem {
  label: string;
  icon?: ReactNode;
  danger?: boolean;
  checked?: boolean;
  disabled?: boolean;
  /** 在这一项上方画分隔线。 */
  divider?: boolean;
  onClick: () => void;
}

const menuCls = "fixed z-50 min-w-40 animate-pop rounded-xl border border-line bg-surface p-1 shadow-xl";

function MenuItems({ items, onPick }: { items: MenuItem[]; onPick: () => void }) {
  return items.map((it) => (
    <div key={it.label}>
      {it.divider && <div className="mx-1 my-1 border-t border-line" />}
      <button
        disabled={it.disabled}
        onClick={() => {
          onPick();
          it.onClick();
        }}
        className={cx(
          "flex w-full items-center gap-2 rounded-lg px-2.5 py-1.5 text-left text-[13px] disabled:opacity-40 disabled:hover:bg-transparent",
          it.danger ? "text-red-600 hover:bg-red-500/10 dark:text-red-400" : "hover:bg-subtle",
        )}
      >
        <span className="flex w-4 justify-center text-muted">
          {it.checked !== undefined ? it.checked && <Check size={14} /> : it.icon}
        </span>
        {it.label}
      </button>
    </div>
  ));
}

/** 浮层打开时，点击浮层外、按 Esc（拦在捕获阶段避免同时关掉所在对话框）、滚动外部容器、窗口失焦或改变大小都关闭浮层。 */
function useDismiss(open: boolean, menu: RefObject<HTMLElement | null>, close: () => void, anchor?: RefObject<HTMLElement | null>) {
  const closeRef = useRef(close);
  closeRef.current = close;
  useEffect(() => {
    if (!open) return;
    const dismiss = () => closeRef.current();
    const onDown = (e: MouseEvent) => {
      const target = e.target as Node;
      if (menu.current?.contains(target) || anchor?.current?.contains(target)) return;
      dismiss();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.stopPropagation();
      e.preventDefault();
      dismiss();
    };
    const onScroll = (e: Event) => {
      if (menu.current?.contains(e.target as Node)) return;
      dismiss();
    };
    window.addEventListener("mousedown", onDown);
    window.addEventListener("keydown", onKey, true);
    window.addEventListener("scroll", onScroll, true);
    window.addEventListener("blur", dismiss);
    window.addEventListener("resize", dismiss);
    return () => {
      window.removeEventListener("mousedown", onDown);
      window.addEventListener("keydown", onKey, true);
      window.removeEventListener("scroll", onScroll, true);
      window.removeEventListener("blur", dismiss);
      window.removeEventListener("resize", dismiss);
    };
  }, [open, menu, anchor]);
}

export function Menu({
  trigger,
  items = [],
  footer,
}: {
  trigger: (open: () => void) => ReactNode;
  items?: MenuItem[];
  /** 菜单底部的额外内容，点击时不关闭菜单，用来放开关。 */
  footer?: ReactNode;
}) {
  const [open, setOpen] = useState(false);
  const anchor = useRef<HTMLSpanElement>(null);
  const menu = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState({ top: 0, left: 0 });

  useLayoutEffect(() => {
    if (!open || !anchor.current || !menu.current) return;
    const a = anchor.current.getBoundingClientRect();
    const m = menu.current.getBoundingClientRect();
    let top = a.bottom + 4;
    if (top + m.height > window.innerHeight - 8) top = a.top - m.height - 4;
    const left = Math.max(8, Math.min(a.right - m.width, window.innerWidth - m.width - 8));
    setPos({ top, left });
  }, [open]);

  useDismiss(open, menu, () => setOpen(false), anchor);

  return (
    <>
      <span ref={anchor} className="inline-flex">
        {trigger(() => setOpen((v) => !v))}
      </span>
      {open &&
        createPortal(
          <div ref={menu} style={pos} className={menuCls}>
            <MenuItems items={items} onPick={() => setOpen(false)} />
            {footer}
          </div>,
          document.body,
        )}
    </>
  );
}

/** 右键菜单：at 为鼠标位置，为 null 时不显示。 */
export function ContextMenu({ at, items, onClose }: { at: { x: number; y: number } | null; items: MenuItem[]; onClose: () => void }) {
  const menu = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState({ top: 0, left: 0 });

  useLayoutEffect(() => {
    if (!at || !menu.current) return;
    const m = menu.current.getBoundingClientRect();
    setPos({
      top: Math.max(8, Math.min(at.y, window.innerHeight - m.height - 8)),
      left: Math.max(8, Math.min(at.x, window.innerWidth - m.width - 8)),
    });
  }, [at]);

  useDismiss(!!at, menu, onClose);

  if (!at) return null;
  return createPortal(
    <div ref={menu} style={pos} className={menuCls} onContextMenu={(e) => e.preventDefault()}>
      <MenuItems items={items} onPick={onClose} />
    </div>,
    document.body,
  );
}
