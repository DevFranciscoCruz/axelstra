import { Accessor, createSignal } from "solid-js";

/** Reading what localStorage (or any saved JSON) holds without trusting its shape: a value that does not pass `ok` gives `fallback`. */
export const parseJson = <T>(raw: string | null, ok: (v: unknown) => v is T, fallback: T): T => {
  if (raw === null) return fallback;
  try {
    const v: unknown = JSON.parse(raw);
    return ok(v) ? v : fallback;
  } catch {
    return fallback;
  }
};

/** the saved JSON under `key`, validated; storage that is missing or throws gives `fallback` */
export const readJson = <T>(key: string, ok: (v: unknown) => v is T, fallback: T): T => {
  try {
    return parseJson(localStorage.getItem(key), ok, fallback);
  } catch {
    return fallback;
  }
};

export const isRecord = (v: unknown): v is Record<string, unknown> => !!v && typeof v === "object" && !Array.isArray(v);
export const isStringArray = (v: unknown): v is string[] => Array.isArray(v) && v.every((x) => typeof x === "string");
/** an array with its non-conforming members dropped */
export const filterArray = <T>(v: unknown, ok: (x: unknown) => x is T): T[] => (Array.isArray(v) ? v.filter(ok) : []);

/** saves `v` as JSON under `key`; storage that is missing, full or blocked is not an error (the value lasts until restart) */
export const writeJson = (key: string, v: unknown): void => {
  try {
    localStorage.setItem(key, JSON.stringify(v));
  } catch {
    /* storage unavailable */
  }
};

/** A signal that remembers itself: starts from the validated saved value (or `fallback`) and saves on every set. */
export function createPersisted<T>(key: string, ok: (v: unknown) => v is T, fallback: T): [Accessor<T>, (v: T | ((prev: T) => T)) => T] {
  const [get, set] = createSignal<T>(readJson(key, ok, fallback));
  return [
    get,
    (v) => {
      const next = set(() => (typeof v === "function" ? (v as (prev: T) => T)(get()) : v));
      writeJson(key, next);
      return next;
    },
  ];
}

/** A boolean setting saved as "1" / "0" (the format the panel toggles already use); anything else gives `def`. */
export function createPersistedFlag(key: string, def: boolean): [Accessor<boolean>, (v: boolean) => void] {
  let initial = def;
  try {
    const raw = localStorage.getItem(key);
    if (raw === "1") initial = true;
    else if (raw === "0") initial = false;
  } catch {
    /* storage unavailable: the default */
  }
  const [get, set] = createSignal(initial);
  return [
    get,
    (v) => {
      set(v);
      try {
        localStorage.setItem(key, v ? "1" : "0");
      } catch {
        /* storage unavailable: lasts until restart */
      }
    },
  ];
}
