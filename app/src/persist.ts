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
