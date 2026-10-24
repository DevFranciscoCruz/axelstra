import { createSignal } from "solid-js";
import { pushNotification } from "./notifications";

export type ToastKind = "info" | "warn" | "error";
/** A button on a toast, e.g. "Desfazer" or "Tentar agora". The toast closes when it is pressed. */
export type ToastAction = { label: string; run: () => void };
export type Toast = {
  id: number;
  /** one toast per key: the same key again replaces the text (and restarts the timer) instead of stacking */
  key: string;
  kind: ToastKind;
  text: string;
  action?: ToastAction;
  /** stays until dismissed (failures, reconnect); the others close after `ttl` unless the pointer or focus is on them */
  sticky: boolean;
  ttl: number;
};
export type ToastInput = {
  text: string;
  kind?: ToastKind;
  action?: ToastAction;
  key?: string;
  /** default: errors and warnings stay, plain notices close by themselves */
  sticky?: boolean;
  ttl?: number;
  /** errors are also logged in the notification center; a transient poll failure opts out */
  record?: boolean;
};

export const NOTICE_TTL_MS = 9000;
/** at most this many toasts on screen: the oldest go first */
export const MAX_TOASTS = 5;

export const [toasts, setToasts] = createSignal<Toast[]>([]);

type Timer = { t: ReturnType<typeof setTimeout>; left: number; at: number };
const timers = new Map<number, Timer>();
let seq = 0;

const stop = (id: number) => {
  const x = timers.get(id);
  if (x) clearTimeout(x.t);
  timers.delete(id);
};
const arm = (id: number, ms: number) => {
  stop(id);
  timers.set(id, { t: setTimeout(() => dismissToast(id), ms), left: ms, at: Date.now() });
};

export const dismissToast = (id: number) => {
  stop(id);
  setToasts((l) => (l.some((x) => x.id === id) ? l.filter((x) => x.id !== id) : l));
};
/** closes the toast of `key` (a poll that works again clears its own failure) */
export const dismissKey = (key: string) => {
  for (const x of toasts()) if (x.key === key) dismissToast(x.id);
};
/** closes every toast whose key starts with `prefix` */
export const dismissPrefix = (prefix: string) => {
  for (const x of toasts()) if (x.key.startsWith(prefix)) dismissToast(x.id);
};
export const pauseToast = (id: number) => {
  const x = timers.get(id);
  if (!x) return;
  clearTimeout(x.t);
  x.left = Math.max(1000, x.left - (Date.now() - x.at));
};
export const resumeToast = (id: number) => {
  const x = timers.get(id);
  if (x) arm(id, x.left);
};

/** Shows a toast and returns its id. Errors and warnings stay until closed; notices close after a while (paused while hovered or focused). */
export const toast = (o: ToastInput): number => {
  const kind = o.kind ?? "info";
  const key = o.key ?? `${kind}:${o.text}`;
  const sticky = o.sticky ?? kind !== "info";
  const ttl = o.ttl ?? NOTICE_TTL_MS;
  const same = toasts().find((x) => x.key === key);
  const id = same?.id ?? ++seq;
  const t: Toast = { id, key, kind, text: o.text, action: o.action, sticky, ttl };
  setToasts((l) => {
    const next = same ? l.map((x) => (x.id === id ? t : x)) : [...l, t];
    for (const gone of next.slice(0, Math.max(0, next.length - MAX_TOASTS))) stop(gone.id);
    return next.slice(-MAX_TOASTS);
  });
  if (sticky) stop(id);
  else arm(id, ttl);
  if (kind === "error" && o.record !== false) pushNotification({ kind: "error", worktree: "", text: o.text.split("\n")[0] });
  return id;
};

/** the newest toast of a kind, as text ("" when none): what the old single-slot `error` / `notice` signals read */
export const latestText = (kind: ToastKind) => [...toasts()].reverse().find((x) => x.kind === kind)?.text ?? "";
