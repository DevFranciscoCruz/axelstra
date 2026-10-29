import { createSignal } from "solid-js";
import type { NotifKind } from "./notifications";
import { isRecord, parseJson, readString, writeJson } from "./persist";

/** What the user is told about, by how much it needs them. */
export type NotifCategory = "need" | "done" | "info" | "error";
export const CATEGORIES: NotifCategory[] = ["need", "done", "info", "error"];

export type NotifyPrefs = Record<NotifCategory, boolean> & {
  /** do not disturb: no system notification, sound or toast; entries are still recorded in the centre */
  dnd: boolean;
};

/** Silent by default except what asks for a person and what failed. */
export const DEFAULT_PREFS: NotifyPrefs = { need: true, done: false, info: false, error: true, dnd: false };

const KEY = "axelstra.notifyPrefs";

/** The category of an event kind; `budget` and CI failures are a failure only at the bad level, so callers may override. */
export const categoryOf = (kind: NotifKind): NotifCategory => {
  switch (kind) {
    case "waiting":
    case "attention":
      return "need";
    case "done":
      return "done";
    case "error":
    case "ci-fail":
      return "error";
    default:
      return "info"; // ci-pass, merged, budget (warn)
  }
};

const isPrefs = (v: unknown): v is Partial<NotifyPrefs> => isRecord(v);

/** saved prefs, tolerating anything: a missing or non-boolean field keeps its default */
export const parsePrefs = (raw: string | null): NotifyPrefs => {
  const v = parseJson<Partial<NotifyPrefs>>(raw, isPrefs, {});
  const out = { ...DEFAULT_PREFS };
  for (const k of [...CATEGORIES, "dnd" as const]) if (typeof v[k] === "boolean") out[k] = v[k] as boolean;
  return out;
};

export type Delivery = {
  /** the entry goes to the centre unread (counts on the bell); false: kept, but already read */
  unread: boolean;
  /** a system notification, a sound and a toast may show */
  interrupt: boolean;
};

/** Pure policy. A category that is off is recorded as read and never interrupts; do-not-disturb records as usual but never interrupts. */
export const deliveryOf = (cat: NotifCategory, p: NotifyPrefs): Delivery => ({
  unread: p[cat],
  interrupt: p[cat] && !p.dnd,
});

export const [notifyPrefs, setPrefsSignal] = createSignal<NotifyPrefs>(parsePrefs(readString(KEY)));

export const setNotifyPref = (key: NotifCategory | "dnd", value: boolean) => {
  const next = { ...notifyPrefs(), [key]: value };
  setPrefsSignal(next);
  writeJson(KEY, next);
};

/** whether an event of this category may interrupt the user right now */
export const mayInterrupt = (cat: NotifCategory) => deliveryOf(cat, notifyPrefs()).interrupt;
